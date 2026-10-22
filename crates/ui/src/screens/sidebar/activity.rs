use super::{HomeSidebar, PAGE};
use crate::screens::thread::view::Clock;
use chrono::{DateTime, Days, NaiveDate};
use gpui::{Action, Bounds, Context, EventEmitter, FocusHandle, Pixels, point, px};
use std::collections::HashSet;

mod options;
mod view;

pub struct ActivityOpened;
impl EventEmitter<ActivityOpened> for HomeSidebar {}

#[derive(Clone, PartialEq, Action)]
#[action(namespace = sidebar, no_json)]
pub struct ArchiveActivity(pub Vec<String>);

struct OptionsMenu {
    focus: FocusHandle,
    previous: Option<FocusHandle>,
    selected: usize,
}

pub(super) struct State {
    priority: bool,
    pinned: bool,
    scheduled: bool,
    archive_supported: bool,
    items: HashSet<String>,
    scheduled_ids: HashSet<String>,
    menu: Option<OptionsMenu>,
    button: Bounds<Pixels>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            priority: true,
            pinned: false,
            scheduled: false,
            archive_supported: true,
            items: HashSet::new(),
            scheduled_ids: HashSet::new(),
            menu: None,
            button: Bounds::default(),
        }
    }
}

impl HomeSidebar {
    pub fn toggle_activity(&mut self, clock: Clock, cx: &mut Context<Self>) {
        self.activity_clock = if self.activity_clock.is_some() {
            None
        } else {
            Some(clock)
        };
        self.activity.items = self
            .threads
            .iter()
            .filter(|(id, _, _)| self.needs_attention(id))
            .map(|(id, _, _)| id.clone())
            .collect();
        self.activity.menu = None;
        self.menu = None;
        self.limit = PAGE;
        self.scroll.set_offset(point(px(0.), px(0.)));
        if self.activity_clock.is_some() {
            cx.emit(ActivityOpened);
        }
        cx.notify();
    }

    pub fn set_activity_archive_supported(&mut self, supported: bool) {
        self.activity.archive_supported = supported;
    }

    pub fn set_scheduled_threads(&mut self, ids: HashSet<String>, cx: &mut Context<Self>) {
        self.activity.scheduled_ids = ids;
        cx.notify();
    }

    pub(super) fn note_activity(&mut self, id: &str) {
        if self.activity_clock.is_some() {
            self.activity.items.insert(id.to_owned());
        }
    }

    fn needs_attention(&self, id: &str) -> bool {
        self.unread.iter().any(|unread| unread == id) || self.running.contains_key(id)
    }

    fn activity_visible(&self, id: &str) -> bool {
        self.activity.scheduled || !self.activity.scheduled_ids.contains(id)
    }

    fn in_priority(&self, id: &str) -> bool {
        self.activity.priority
            && self.activity.items.contains(id)
            && (!self.activity.pinned || !self.pinned.contains(id))
    }

    fn in_recent(&self, id: &str, at: i64, oldest: NaiveDate, offset: i64) -> bool {
        self.activity_visible(id)
            && !self.in_priority(id)
            && (!self.activity.pinned || !self.pinned.contains(id))
            && (day(at, offset) >= oldest || self.activity.items.contains(id))
    }

    pub(super) fn activity_window(&self) -> Option<(usize, bool)> {
        let clock = self.activity_clock?;
        let oldest = day(clock.now(), clock.utc_offset) - Days::new(6);
        let recent = self
            .threads
            .iter()
            .filter(|(id, _, at)| self.in_recent(id, *at, oldest, clock.utc_offset))
            .count();
        let has_more = self.has_more
            && self
                .threads
                .iter()
                .all(|(_, _, at)| day(*at, clock.utc_offset) >= oldest);
        Some((recent, has_more))
    }
}

fn day(at: i64, offset: i64) -> NaiveDate {
    DateTime::from_timestamp(at + offset, 0)
        .unwrap_or_default()
        .date_naive()
}
