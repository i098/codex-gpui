use crate::{Root, host, windows};
use gpui::{App, AppContext as _, Global, PromptLevel};
use std::borrow::Cow;

#[derive(Default)]
struct QuitPending(bool);
impl Global for QuitPending {}

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
    if handles.is_empty() {
        cx.quit();
        return;
    }
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
        let answer = cx.update(|cx| {
            let Some(first) = windows::handles(windows::Kind::Primary, cx)
                .first()
                .copied()
            else {
                cx.quit();
                return None;
            };
            let detail = match result {
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
                    Cow::Borrowed(detail)
                }
                Err(error) => Cow::Owned(format!("Could not check scheduled tasks: {error}")),
            };
            first
                .update(cx, |_, window, cx| {
                    window.prompt(
                        PromptLevel::Warning,
                        "Quit Codex?",
                        Some(detail.as_ref()),
                        &["Quit", "Cancel"],
                        cx,
                    )
                })
                .ok()
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
