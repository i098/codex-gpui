use super::*;
use gpui::Div;
use ui::screens::sidebar::sections::{SectionAction, SectionCommand};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SectionPage {
    data: Vec<Section>,
    next_cursor: Option<String>,
}

#[derive(Deserialize)]
struct Section {
    id: String,
    name: String,
}

pub(super) fn actions(root: Div, cx: &mut Context<Root>) -> Div {
    root.on_action(cx.listener(|this, action: &SectionAction, window, cx| {
        this.section_action(action, window, cx);
    }))
}

impl Root {
    pub(crate) fn load_sections(&self, cx: &mut Context<Self>) {
        let bridge = self.bridge.clone();
        let task = cx.spawn(async move |this, cx| -> Result<()> {
            let mut sections = Vec::new();
            let mut threads = Vec::new();
            let mut cursor: Option<String> = None;
            loop {
                let page: SectionPage = bridge.request("threadSection/list", json!({"cursor": cursor, "limit": 100})).await?;
                for section in page.data.into_iter().filter(|s| s.id != PINNED_SECTION_ID) {
                    let mut members = Vec::new();
                    let mut cursor: Option<String> = None;
                    loop {
                        let page: ThreadListResponse = bridge.request("thread/list", json!({
                            "cursor": cursor, "limit": 100, "sectionId": section.id,
                            "sortKey": "section_position", "modelProviders": [], "useStateDbOnly": true
                        })).await?;
                        members.extend(page.data.iter().map(|t| format!("codex:thread:local:{}", t.id)));
                        threads.extend(page.data);
                        cursor = page.next_cursor;
                        if cursor.is_none() { break; }
                    }
                    sections.push(json!({"id": section.id, "name": section.name, "itemKeys": members}));
                }
                cursor = page.next_cursor;
                if cursor.is_none() { break; }
            }
            this.update(cx, |this, cx| -> Result<()> {
                if this.signed_out() || this.sidebar.read(cx).has_sections() {
                    return Ok(());
                }
                for thread in threads {
                    if !this.threads.iter().any(|t| t.id == thread.id) { this.threads.push(thread); }
                }
                this.sidebar.update(cx, |sidebar, cx| {
                    sidebar.set_threads(&this.threads, cx);
                    sidebar.set_sections(&json!({"sections": sections, "collapsedSectionIds": []}), cx)
                })?;
                Ok(())
            })??;
            Ok(())
        });
        log_err(task, cx);
    }

    fn section_action(
        &mut self,
        action: &SectionAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let id = &action.section;
        if self.sidebar.read(cx).section_name(id).is_none() {
            return;
        }
        match action.command {
            SectionCommand::NewChat => {
                let members = self.sidebar.read(cx).section_threads(id);
                let selected = self.state.project_root.as_deref();
                let cwd =
                    self.threads
                        .iter()
                        .filter(|t| {
                            members.contains(&t.id)
                                && self.sidebar.read(cx).projects().iter().any(|p| {
                                    !p.missing && p.root.as_deref() == Some(t.cwd.as_str())
                                })
                        })
                        .max_by_key(|t| (Some(t.cwd.as_str()) == selected, t.updated_at))
                        .map(|t| t.cwd.clone());
                self.state.project_root = cwd;
                let project = self.state.project_root.as_deref().map(project_name);
                self.home.update(cx, |home, cx| {
                    home.project = project;
                    cx.notify();
                });
                let roots = self.state.project_root.iter().cloned().collect();
                let composer = self.home.read(cx).composer.clone();
                composer.update(cx, |c, cx| c.set_search_roots(roots, cx));
                self.new_chat(window, cx);
                self.new_chat_section = Some(id.clone());
                self.sidebar.update(cx, |s, cx| s.expand_section(id, cx));
            }
            SectionCommand::Edit | SectionCommand::New => {
                if self.dialog.is_some() {
                    return;
                }
                let editing = action.command == SectionCommand::Edit;
                let name = if editing {
                    self.sidebar
                        .read(cx)
                        .section_name(id)
                        .unwrap_or_default()
                        .to_owned()
                } else {
                    String::new()
                };
                let dialog = cx.new(|cx| Dialog::section(editing.then(|| id.clone()), name, cx));
                self.show_dialog(dialog, window, cx);
            }
            SectionCommand::Remove => self.delete_section(id.clone(), cx),
            SectionCommand::MarkRead => self.mark_section_read(id.clone(), cx),
            SectionCommand::Archive => self.confirm_section_archive(id.clone(), window, cx),
        }
    }

    pub(crate) fn save_section(
        &mut self,
        id: Option<String>,
        name: String,
        cx: &mut Context<Self>,
    ) {
        let bridge = self.bridge.clone();
        let task = cx.spawn(async move |this, cx| -> Result<()> {
            let result = if let Some(id) = id {
                bridge
                    .request::<_, Value>(
                        "threadSection/update",
                        json!({"sectionId": id, "name": name}),
                    )
                    .await
                    .map(|_| id)
            } else {
                bridge
                    .request::<_, Value>("threadSection/create", json!({"name": name}))
                    .await
                    .and_then(|v| {
                        v["section"]["id"]
                            .as_str()
                            .map(str::to_owned)
                            .context("section create response has no id")
                    })
            };
            this.update(cx, |this, cx| match &result {
                Ok(id) => this
                    .sidebar
                    .update(cx, |s, cx| s.update_section(id, &name, cx)),
                Err(_) => this.toast(Toast::danger("Couldn't save your changes. Try again"), cx),
            })?;
            result.map(drop)
        });
        log_err(task, cx);
    }

    fn delete_section(&mut self, id: String, cx: &mut Context<Self>) {
        let bridge = self.bridge.clone();
        let task = cx.spawn(async move |this, cx| -> Result<()> {
            let result = bridge
                .request::<_, Value>("threadSection/delete", json!({"sectionId": id}))
                .await;
            this.update(cx, |this, cx| {
                if result.is_ok() {
                    for thread in &mut this.threads {
                        if thread.section.as_ref().is_some_and(|s| s.id == id) {
                            thread.section = None;
                        }
                    }
                    if this.new_chat_section.as_deref() == Some(&id) {
                        this.new_chat_section = None;
                    }
                    this.sidebar.update(cx, |s, cx| s.remove_section(&id, cx));
                } else {
                    this.toast(Toast::danger("Couldn't remove section. Try again"), cx);
                }
            })?;
            result.map(drop)
        });
        log_err(task, cx);
    }

    fn mark_section_read(&mut self, id: String, cx: &mut Context<Self>) {
        self.sidebar.update(cx, |sidebar, cx| {
            let members = sidebar.section_threads(&id);
            sidebar.unread.retain(|thread| !members.contains(thread));
            cx.notify();
        });
    }

    fn confirm_section_archive(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        if self.dialog.is_some() {
            return;
        }
        let members = self.sidebar.read(cx).section_threads(&id);
        if members.is_empty() {
            return;
        }
        let running = self
            .threads
            .iter()
            .any(|t| members.contains(&t.id) && t.status == ThreadStatus::Active);
        let name = self
            .sidebar
            .read(cx)
            .section_name(&id)
            .unwrap_or_default()
            .to_owned();
        let dialog = cx.new(|cx| Dialog::archive_section(id, name, members.len(), running, cx));
        self.show_dialog(dialog, window, cx);
    }

    pub(crate) fn archive_section(
        &mut self,
        id: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let members = self.sidebar.read(cx).section_threads(&id);
        let bridge = self.bridge.clone();
        let task = cx.spawn_in(window, async move |this, cx| -> Result<()> {
            let mut failed = 0;
            let mut archived = 0;
            for thread_id in members {
                if bridge
                    .request::<_, Value>("thread/archive", json!({"threadId": thread_id}))
                    .await
                    .is_ok()
                {
                    archived += 1;
                    this.update_in(cx, |this, window, cx| {
                        this.threads.retain(|t| t.id != thread_id);
                        this.sidebar.update(cx, |s, cx| {
                            s.remove_section_thread(&thread_id, cx);
                            s.set_threads(&this.threads, cx);
                        });
                        if this.state.route == Route::Thread(thread_id) {
                            this.new_chat(window, cx);
                            this.new_chat_section = Some(id.clone());
                        }
                    })?;
                } else {
                    failed += 1;
                }
            }
            this.update(cx, |this, cx| {
                if failed == 0 {
                    this.toast(
                        Toast::success(format!(
                            "Archived {archived} {}",
                            if archived == 1 { "task" } else { "tasks" }
                        )),
                        cx,
                    );
                } else {
                    this.toast(
                        Toast::danger(format!("Archived {archived} chats; {failed} failed")),
                        cx,
                    );
                }
            })?;
            Ok(())
        });
        log_err(task, cx);
    }
}
