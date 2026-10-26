use super::{HomeSidebar, ListState, PAGE};
use crate::screens::thread::view::Clock;
use chrono::{DateTime, Days, NaiveDate};
use gpui::{Action, Context, EventEmitter, FocusHandle, point, px};
use std::collections::HashSet;

mod options;
mod view;

pub struct ActivityOpened;
impl EventEmitter<ActivityOpened> for HomeSidebar {}

#[derive(Clone, PartialEq, Action)]
#[action(namespace = sidebar, no_json)]
pub struct ArchiveActivity(pub Vec<String>);

pub(super) struct OptionsMenu {
    focus: FocusHandle,
    previous: Option<FocusHandle>,
    selected: usize,
}

impl HomeSidebar {
    pub fn toggle_activity(&mut self, clock: Clock, cx: &mut Context<Self>) {
        self.activity_clock = if self.activity_clock.is_some() {
            None
        } else {
            Some(clock)
        };
        self.activity_items.clear();
        self.activity_recency.clear();
        self.activity_scheduled_ids.clear();
        self.activity_sources_state = ListState::Ready;
        self.activity_menu = None;
        self.menu = None;
        self.limit = PAGE;
        self.scroll.set_offset(point(px(0.), px(0.)));
        if self.activity_clock.is_some() {
            self.activity_sources_state = ListState::Loading;
            self.activity_started_at = clock.now();
            for (id, _, _) in &self.threads {
                if self.unread.contains(id) || self.running.contains_key(id) {
                    self.activity_items.insert(id.clone());
                }
            }
            self.remember_activity_recency();
            cx.emit(ActivityOpened);
        }
        cx.notify();
    }

    pub fn set_activity_archive_supported(&mut self, supported: bool) {
        self.activity_archive_supported = supported;
    }

    pub fn set_scheduled_threads(&mut self, ids: HashSet<String>, cx: &mut Context<Self>) {
        self.activity_scheduled_ids = ids;
        self.activity_sources_state = ListState::Ready;
        cx.notify();
    }

    pub(super) fn note_activity(&mut self, id: &str) {
        if self.activity_clock.is_some() {
            self.activity_items.insert(id.to_owned());
        }
    }

    fn needs_attention(&self, id: &str) -> bool {
        self.unread.iter().any(|unread| unread == id) || self.running.contains_key(id)
    }

    fn activity_visible(&self, id: &str) -> bool {
        self.activity_scheduled || !self.activity_scheduled_ids.contains(id)
    }

    fn in_priority(&self, id: &str) -> bool {
        self.activity_priority
            && self.activity_items.contains(id)
            && (!self.activity_pinned || !self.pinned.contains(id))
    }

    fn in_recent(&self, id: &str, at: i64, oldest: NaiveDate, offset: i64) -> bool {
        self.activity_visible(id)
            && !self.in_priority(id)
            && (!self.activity_pinned || !self.pinned.contains(id))
            && (local_date(self.activity_updated_at(id, at), offset) >= oldest
                || self.activity_items.contains(id))
    }

    pub(super) fn activity_window(&self) -> Option<(usize, bool)> {
        let clock = self.activity_clock?;
        let oldest = local_date(self.activity_started_at, clock.utc_offset) - Days::new(6);
        let recent = self
            .threads
            .iter()
            .filter(|(id, _, at)| self.in_recent(id, *at, oldest, clock.utc_offset))
            .count();
        let has_more = self.has_more
            && self
                .threads
                .iter()
                .all(|(_, _, at)| local_date(*at, clock.utc_offset) >= oldest);
        Some((recent, has_more))
    }

    pub(super) fn remember_activity_recency(&mut self) {
        let Some(clock) = self.activity_clock else {
            return;
        };
        let oldest = local_date(self.activity_started_at, clock.utc_offset) - Days::new(6);
        for (id, _, at) in &self.threads {
            if self.in_recent(id, *at, oldest, clock.utc_offset) {
                self.activity_recency.entry(id.clone()).or_insert(*at);
            }
        }
    }

    fn activity_updated_at(&self, id: &str, current: i64) -> i64 {
        self.activity_recency.get(id).copied().unwrap_or(current)
    }
}

fn local_date(at: i64, offset: i64) -> NaiveDate {
    DateTime::from_timestamp(at + offset, 0)
        .unwrap_or_default()
        .date_naive()
}

#[cfg(test)]
mod tests {
    use super::*;
    use bridge::types::Thread;
    use gpui::AppContext as _;

    #[gpui::test]
    fn priority_snapshot_retains_read_items_and_recency_until_reopened(
        cx: &mut gpui::TestAppContext,
    ) {
        let now = 1_792_083_600;
        let make_thread = |id: &str, updated_at| {
            serde_json::from_value::<Thread>(serde_json::json!({
                "id": id, "preview": id, "modelProvider": "test",
                "cwd": "/tmp/activity-test", "createdAt": 0, "updatedAt": updated_at,
                "status": {"type": "idle"}, "turns": []
            }))
            .unwrap()
        };
        let mut threads = [
            make_thread("recent", now),
            make_thread("old-unread", now - 8 * 86_400),
            make_thread("old-idle", now - 8 * 86_400),
        ];
        let sidebar = cx.new(|_| HomeSidebar::new(&threads, None));
        let clock = Clock {
            utc_offset: -7 * 3600,
            pinned_now: Some(now),
            elapsed_since: None,
        };
        sidebar.update(cx, |sidebar, cx| {
            sidebar.set_unread("old-unread", cx);
            sidebar.toggle_activity(clock, cx);
            assert!(sidebar.in_priority("old-unread"));
            assert!(!sidebar.in_priority("old-idle"));
            assert_eq!(sidebar.activity_window(), Some((1, false)));

            sidebar.select(Some("old-unread"), cx);
            assert!(sidebar.unread.is_empty());
            assert!(sidebar.in_priority("old-unread"));

            sidebar.set_running("old-idle", true, cx);
            assert!(sidebar.in_priority("old-idle"));
            sidebar.set_running("old-idle", false, cx);
            assert!(sidebar.in_priority("old-idle"));

            threads[0].updated_at = now - 9 * 86_400;
            sidebar.set_threads(&threads, cx);
            assert_eq!(sidebar.activity_window(), Some((1, false)));
            assert_eq!(
                sidebar.activity_updated_at("recent", threads[0].updated_at),
                now
            );
            threads[0].updated_at = now;
            sidebar.set_threads(&threads, cx);

            sidebar.toggle_activity(clock, cx);
            sidebar.toggle_activity(clock, cx);
            assert!(!sidebar.in_priority("old-unread"));
            assert!(!sidebar.in_priority("old-idle"));
            assert_eq!(sidebar.activity_window(), Some((1, false)));

            let mut created = make_thread("new-idle", now);
            created.created_at = now;
            sidebar.add_thread(&created, cx);
            assert!(sidebar.in_priority("new-idle"));
        });
    }
}
