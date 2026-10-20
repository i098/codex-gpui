use super::*;

#[derive(Clone, Copy, PartialEq)]
pub enum SectionCommand {
    NewChat,
    Edit,
    MarkRead,
    Archive,
    New,
    Remove,
}

#[derive(Clone, PartialEq, Action)]
#[action(namespace = sidebar, no_json)]
pub struct SectionAction {
    pub section: String,
    pub command: SectionCommand,
}

impl HomeSidebar {
    pub fn has_sections(&self) -> bool {
        !self.sections.is_empty()
    }

    pub(super) fn section(&self, id: &str) -> Option<&CustomSection> {
        self.sections
            .iter()
            .find(|s| s.id.strip_prefix("custom:") == Some(id))
    }

    fn section_mut(&mut self, id: &str) -> Option<&mut CustomSection> {
        self.sections
            .iter_mut()
            .find(|s| s.id.strip_prefix("custom:") == Some(id))
    }

    pub fn section_name(&self, id: &str) -> Option<&str> {
        self.section(id).map(|s| s.title.as_ref())
    }

    pub fn section_threads(&self, id: &str) -> Vec<String> {
        self.section(id)
            .into_iter()
            .flat_map(|s| &s.rows)
            .map(|&row| self.threads[row].0.clone())
            .collect()
    }

    pub fn update_section(&mut self, id: &str, name: &str, cx: &mut Context<Self>) {
        if let Some(section) = self.section_mut(id) {
            section.title = name.to_owned().into();
        } else {
            self.sections.push(CustomSection::new(
                id.to_owned(),
                name.to_owned(),
                Vec::new(),
                false,
            ));
        }
        cx.notify();
    }

    pub fn remove_section(&mut self, id: &str, cx: &mut Context<Self>) {
        self.sections
            .retain(|s| s.id.strip_prefix("custom:") != Some(id));
        self.reindex_sections();
        cx.notify();
    }

    pub fn expand_section(&mut self, id: &str, cx: &mut Context<Self>) {
        if let Some(section) = self.section_mut(id) {
            section.collapsed = false;
        }
        cx.notify();
    }

    pub fn remove_section_thread(&mut self, thread: &str, cx: &mut Context<Self>) {
        for section in &mut self.sections {
            section.thread_ids.retain(|id| id != thread);
        }
        self.reindex_sections();
        cx.notify();
    }

    pub fn clear_sections(&mut self, cx: &mut Context<Self>) {
        self.sections.clear();
        self.grouped_rows.clear();
        self.menu = None;
        cx.notify();
    }

    pub fn assign_section(&mut self, thread: &str, id: &str, cx: &mut Context<Self>) {
        for section in &mut self.sections {
            section.thread_ids.retain(|t| t != thread);
            if section.id.strip_prefix("custom:") == Some(id) {
                section.thread_ids.insert(0, thread.to_owned());
                section.collapsed = false;
            }
        }
        self.reindex_sections();
        cx.notify();
    }

    pub(super) fn section_actions(
        &self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let section = &self.sections[index];
        let id = section
            .id
            .strip_prefix("custom:")
            .unwrap_or(&section.id)
            .to_owned();
        let header = focus::handle(section.id.clone(), -1, window, cx);
        let options = focus::handle(format!("section-options:{id}"), -1, window, cx);
        let new_chat = focus::handle(format!("section-new-chat:{id}"), -1, window, cx);
        let focused = [&header, &options, &new_chat]
            .iter()
            .any(|f| f.read(cx).handle.is_focused(window))
            || self.section_menu_open(&id);
        let button = |key: String,
                      label: String,
                      icon_name: &'static str,
                      handle: &gpui::Entity<focus::ControlFocus>| {
            focus::ring(
                div()
                    .id(key)
                    .role(gpui::Role::Button)
                    .aria_label(label)
                    .size(px(24.))
                    .rounded(px(8.))
                    .cursor_pointer()
                    .flex()
                    .items_center()
                    .justify_center()
                    .hover(|s| s.bg(cx.theme().ghost_hover))
                    .child(icon(icon_name, 16., cx.theme().text_tertiary)),
                handle,
                px(8.),
                px(0.),
                cx,
            )
        };
        let menu_id = id.clone();
        let new_id = id.clone();
        div()
            .absolute()
            .right(px(6.))
            .top(px(3.))
            .flex()
            .gap(px(4.))
            .when(!focused, |d| d.invisible())
            .group_hover(section.id.clone(), |s| s.visible())
            .child(
                button(
                    format!("section-options:{id}"),
                    format!("Options for {}", section.title),
                    "diff-more",
                    &options,
                )
                .on_click(cx.listener(
                    move |this, event: &gpui::ClickEvent, window, cx| {
                        this.open_section_menu(menu_id.clone(), event.position(), window, cx);
                    },
                )),
            )
            .child(
                button(
                    format!("section-new-chat:{id}"),
                    format!("New chat in {}", section.title),
                    "new-chat",
                    &new_chat,
                )
                .on_click(move |_, window, cx| {
                    cx.stop_propagation();
                    window.dispatch_action(
                        Box::new(SectionAction {
                            section: new_id.clone(),
                            command: SectionCommand::NewChat,
                        }),
                        cx,
                    );
                }),
            )
            .into_any_element()
    }
}
