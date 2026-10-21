use crate::{Root, host, windows};
use gpui::{
    AnyWindowHandle, App, AppContext as _, Context, Global, KeyDownEvent, PromptLevel, Window,
};

#[derive(Default)]
struct QuitPending(bool);
impl Global for QuitPending {}

pub(crate) fn actions(root: gpui::Div, cx: &mut gpui::Context<Root>) -> gpui::Div {
    use gpui::InteractiveElement as _;
    root.capture_action(cx.listener(|root, _: &ui::composer::Dismiss, window, cx| {
        if !root.dismiss_shell_overlay(window, cx) {
            cx.propagate();
        }
    }))
    .capture_key_down(cx.listener(|root, event: &KeyDownEvent, window, cx| {
        if event.keystroke.key == "escape" && root.dismiss_shell_overlay(window, cx) {
            cx.stop_propagation();
        }
    }))
    .on_action(cx.listener(|root, action: &ui::PreviewSidebar, _, cx| {
        let sidebar = match action.0 {
            "home" => root.sidebar.clone().into(),
            "scheduled" => {
                if root.scheduled.is_none() {
                    let tasks = root.tasks();
                    root.scheduled = Some(cx.new(|cx| {
                        ui::screens::automations::ScheduledSidebar::new(
                            tasks,
                            None,
                            crate::utc_offset(),
                            cx,
                        )
                    }));
                    root.load_automations(cx);
                }
                root.scheduled.as_ref().unwrap().clone().into()
            }
            _ => return,
        };
        root.shell()
            .clone()
            .update(cx, |shell, cx| shell.set_peek_sidebar(sidebar, cx));
    }))
}

impl Root {
    fn dismiss_shell_overlay(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if self.dialog.is_some() || self.command_menu.is_some() {
            return false;
        }
        let composer = self.composer(cx);
        let composer = composer.read(cx);
        if composer.menu_state(window).0 || composer.context_menu_at().is_some() {
            return false;
        }
        self.shell()
            .clone()
            .update(cx, |shell, cx| shell.dismiss_overlay(window, cx))
    }
}

pub(crate) fn request_quit(cx: &mut App) {
    if cx
        .try_global::<QuitPending>()
        .is_some_and(|pending| pending.0)
    {
        return;
    }
    cx.set_global(QuitPending(true));
    cx.defer(begin_quit);
}

fn begin_quit(cx: &mut App) {
    let handles = windows::handles(windows::Kind::Primary, cx);
    let Some(first) = handles.first().copied() else {
        cx.quit();
        return;
    };
    let mut hosts = Vec::new();
    for handle in handles {
        if let Some(root) = handle.downcast::<Root>() {
            let _ = root.update(cx, |root, _, _| {
                if !hosts
                    .iter()
                    .any(|host| std::sync::Arc::ptr_eq(host, &root.host))
                {
                    hosts.push(root.host.clone());
                }
            });
        }
    }
    let read = cx.background_spawn(async move {
        let mut scheduled = false;
        for host in hosts {
            let response = host.fetch(&host::Request::post("list-automations", None))?;
            let value = host::json_body(&response)?;
            scheduled |= value["items"]
                .as_array()
                .is_some_and(|items| items.iter().any(|item| item["status"] == "ACTIVE"));
        }
        anyhow::Ok(scheduled)
    });
    cx.spawn(async move |cx| {
        let result = read.await;
        let answer = cx.update(|cx| match result {
            Ok(scheduled) => {
                let active = windows::handles(windows::Kind::Primary, cx)
                    .into_iter()
                    .any(|handle| {
                        handle
                            .downcast::<Root>()
                            .and_then(|handle| {
                                handle
                                    .update(cx, |root, _, _| {
                                        root.threads.iter().any(|thread| {
                                            thread.status == bridge::types::ThreadStatus::Active
                                        })
                                    })
                                    .ok()
                            })
                            .unwrap_or(false)
                    });
                let Some(detail) = quit_detail(active, scheduled) else {
                    cx.quit();
                    return None;
                };
                first
                    .update(cx, |_, window, cx| {
                        window.prompt(
                            PromptLevel::Warning,
                            "Quit Codex?",
                            Some(detail),
                            &["Quit", "Cancel"],
                            cx,
                        )
                    })
                    .ok()
            }
            Err(error) => {
                show_quit_error(first, error, cx);
                None
            }
        });
        if let Some(answer) = answer
            && answer.await == Ok(0)
        {
            cx.update(|cx| cx.quit());
        }
        cx.update(|cx| cx.set_global(QuitPending(false)));
    })
    .detach();
}

fn show_quit_error(handle: AnyWindowHandle, error: anyhow::Error, cx: &mut App) {
    if let Some(root) = handle.downcast::<Root>() {
        let _ = root.update(cx, |root, _, cx| {
            root.toast(
                ui::toast::Toast::danger("Could not check scheduled tasks")
                    .description(error.to_string()),
                cx,
            );
        });
    }
}

fn quit_detail(active: bool, scheduled: bool) -> Option<&'static str> {
    match (active, scheduled) {
        (false, false) => None,
        (true, false) => Some("Active local chats on this machine will be interrupted"),
        (false, true) => Some("Scheduled tasks won't run while Codex is closed"),
        (true, true) => Some(
            "Active local chats on this machine will be interrupted and scheduled tasks won't run while Codex is closed",
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::quit_detail;

    #[test]
    fn quit_confirmation_is_required_when_work_will_be_interrupted() {
        for active in [false, true] {
            for scheduled in [false, true] {
                assert_eq!(
                    quit_detail(active, scheduled).is_some(),
                    active || scheduled
                );
            }
        }
    }
}
