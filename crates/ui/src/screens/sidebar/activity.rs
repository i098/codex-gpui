use super::*;
use crate::screens::thread::view::Clock;
use chrono::{DateTime, Days, NaiveDate};

impl HomeSidebar {
    pub fn toggle_activity(&mut self, clock: Clock, cx: &mut Context<Self>) {
        self.activity_clock = if self.activity_clock.is_some() {
            None
        } else {
            Some(clock)
        };
        self.menu = None;
        self.limit = PAGE;
        self.scroll.set_offset(point(px(0.), px(0.)));
        cx.notify();
    }

    fn needs_attention(&self, id: &str) -> bool {
        self.unread.iter().any(|unread| unread == id) || self.running.contains_key(id)
    }

    pub(super) fn activity_window(&self) -> Option<(usize, bool)> {
        let clock = self.activity_clock?;
        let oldest = day(clock.now(), clock.utc_offset) - Days::new(6);
        let recent = self
            .threads
            .iter()
            .filter(|(id, _, at)| !self.needs_attention(id) && day(*at, clock.utc_offset) >= oldest)
            .count();
        let has_more = self.has_more
            && self
                .threads
                .iter()
                .all(|(_, _, at)| day(*at, clock.utc_offset) >= oldest);
        Some((recent, has_more))
    }

    pub(super) fn activity_list(&self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let clock = self.activity_clock.expect("activity view");
        let today = day(clock.now(), clock.utc_offset);
        let oldest = today - Days::new(6);
        let priority = self
            .threads
            .iter()
            .filter(|(id, _, _)| self.needs_attention(id));
        let empty = priority.clone().next().is_none();
        let mut list = div()
            .px(px(8.))
            .flex()
            .flex_col()
            .child(heading("Priority", cx));
        if empty {
            list = list.child(match self.threads_state {
                ListState::Loading => {
                    sidebar_empty("Checking unread chats…", cx).into_any_element()
                }
                ListState::Failed => load_error::<RetryThreads>(
                    "Unread chats couldn't be loaded",
                    "retry-activity",
                    window,
                    cx,
                ),
                ListState::Ready => sidebar_empty("Nothing needs attention", cx).into_any_element(),
            });
        } else {
            list = list.child(div().flex().flex_col().gap(px(1.)).children(
                priority.map(|(id, label, _)| self.chat_row(id, label.clone(), window, cx)),
            ));
        }
        let mut recent: Vec<_> = self
            .threads
            .iter()
            .filter(|(id, _, at)| !self.needs_attention(id) && day(*at, clock.utc_offset) >= oldest)
            .collect();
        recent.sort_by_key(|(_, _, at)| std::cmp::Reverse(*at));
        let mut previous = None;
        for (id, label, at) in recent.into_iter().take(self.limit) {
            let date = day(*at, clock.utc_offset);
            if previous != Some(date) {
                let title = if date == today {
                    "Today".to_owned()
                } else if date == today - Days::new(1) {
                    "Yesterday".to_owned()
                } else {
                    date.format("%A").to_string()
                };
                list = list.child(div().h(px(16.))).child(heading(title, cx));
                previous = Some(date);
            }
            list = list.child(self.chat_row(id, label.clone(), window, cx).mb(px(1.)));
        }
        list.children(self.page_tail(cx)).into_any_element()
    }
}

fn day(at: i64, offset: i64) -> NaiveDate {
    DateTime::from_timestamp(at + offset, 0)
        .unwrap_or_default()
        .date_naive()
}

fn heading(label: impl Into<SharedString>, cx: &App) -> Div {
    div()
        .h(px(30.))
        .pl(px(8.))
        .flex()
        .items_center()
        .child(text(label, 14., 21., 500., cx.theme().text_tertiary))
}
