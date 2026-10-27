use crate::*;
use gpui::Div;

pub(super) fn actions(root: Div, cx: &mut Context<Root>) -> Div {
    root.on_action(cx.listener(|this, _: &OpenScheduled, window, cx| {
        this.navigate(Route::Scheduled, window, cx)
    }))
    .on_action(
        cx.listener(|this, a: &automations::OpenAutomation, window, cx| {
            this.navigate(Route::Automation(a.id.clone()), window, cx)
        }),
    )
    .on_action(
        cx.listener(|this, a: &automations::RunAutomationNow, window, cx| {
            this.run_automation_now(a.id.clone(), window, cx)
        }),
    )
    .on_action(
        cx.listener(|this, action: &automations::ToggleAutomation, _, cx| {
            this.toggle_automation(&action.id, cx);
        }),
    )
    .on_action(
        cx.listener(|this, action: &automations::ShareAutomation, _, cx| {
            this.share_automation(&action.id, cx);
        }),
    )
    .on_action(cx.listener(|this, action: &automations::EditAutomation, window, cx| {
        this.edit_automation(Some(action.id.clone()), window, cx);
    }))
    .on_action(cx.listener(|this, action: &automations::CreateAutomation, window, cx| {
        match action.mode {
            automations::CreationMode::Manual => this.edit_automation(None, window, cx),
            automations::CreationMode::Codex | automations::CreationMode::ChatGPT => {
                this.new_chat(window, cx);
                let composer = this.home.read(cx).composer.clone();
                composer.update(cx, |composer, cx| composer.set_text(
                    "Let's set up a scheduled task together. First, explain how scheduled tasks work in ChatGPT. Then interview me to figure out what I need scheduled and when it should run.", cx,
                ));
            }
        }
    }))
    .on_action(cx.listener(|this, _: &automations::NewTask, window, cx| {
        this.open_dialog(DialogKind::ScheduleTask, window, cx)
    }))
    .on_action(
        cx.listener(|this, _: &automations::StartSuggestedTask, _, cx| {
            this.toast(
                Toast::danger("Starting suggested tasks isn’t supported in this version"),
                cx,
            )
        }),
    )
}

pub(super) fn scheduled(root: &mut Root, cx: &mut Context<Root>) -> Entity<Shell> {
    let tasks = root.tasks();
    root.load_automations(cx);
    let (shell, sidebar, page) = automations::shell(tasks, None, utc_offset(), cx);
    root.scheduled = Some(sidebar);
    root.sync_starting(cx);
    root.suggestions = page;
    shell
}

pub(super) fn automation(
    root: &mut Root,
    id: String,
    window: &mut Window,
    cx: &mut Context<Root>,
) -> Entity<Shell> {
    // The list is the one the Scheduled page read; a cold start reads it now.
    if root.automations.is_none() && root.automation_load.is_none() {
        root.load_automations(cx);
    }
    let tasks = root.tasks();
    let selected = tasks.iter().find(|t| t.id == id).cloned().map(|task| {
        let composer: AnyView = cx.new(|cx| Composer::automation(window, cx)).into();
        (task, composer)
    });
    let (shell, sidebar, page) = automations::shell(tasks, selected, utc_offset(), cx);
    root.scheduled = Some(sidebar);
    root.sync_starting(cx);
    root.suggestions = page;
    shell
}

pub(super) fn thread(root: &mut Root, cx: &mut Context<Root>) -> Entity<Shell> {
    let tasks = root.tasks();
    let task = tasks
        .iter()
        .find(|t| Some(&t.id) == root.sidebar_automation.as_ref())
        .cloned();
    let selected = task.map(|task| (task, cx.new(|_| ui::EmptyMain).into()));
    let (shell, sidebar, _) = automations::shell(tasks, selected, utc_offset(), cx);
    root.scheduled = Some(sidebar);
    root.sync_starting(cx);
    shell
}

impl Root {
    /// Builds the original local task URL with a query-encoded automation ID.
    pub fn automation_link(id: &str) -> String {
        let mut link = url::Url::parse("https://chatgpt.com/scheduled").expect("constant URL");
        link.query_pairs_mut()
            .append_pair("automationId", id)
            .append_pair("automationSource", "local");
        link.into()
    }

    fn share_automation(&mut self, id: &str, cx: &mut Context<Self>) {
        let link = Self::automation_link(id);
        cx.write_to_clipboard(gpui::ClipboardItem::new_string(link.clone()));
        // The original confirms only a successful clipboard write.
        if cx
            .read_from_clipboard()
            .and_then(|item| item.text())
            .as_deref()
            == Some(&link)
        {
            self.toast(Toast::success("Link copied"), cx);
        } else {
            eprintln!("codex-gpui: share task: the clipboard did not take the link");
        }
    }
}
