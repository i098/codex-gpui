use super::{AppServer, SettingsPage};
use crate::{
    confirmation::{Confirmation, Outcome},
    icon::icon,
    text,
    toast::{Toast, Toaster},
};
use bridge::storage::{self, Candidate, Entry, Usage};
use gpui::{
    App, AppContext as _, BorrowAppContext as _, Context, Entity, FocusHandle, Global,
    InteractiveElement, IntoElement, KeyDownEvent, MouseButton, ParentElement, Render,
    SharedString, StatefulInteractiveElement, Styled, Window, anchored, deferred, div, point,
    prelude::FluentBuilder as _, px, relative, rgb,
};
use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
};
use theme::{ActiveTheme as _, Appearance};

#[derive(Default)]
struct CleanupState {
    deleting: bool,
}

impl Global for CleanupState {}

struct Review {
    entries: Vec<Entry>,
    rows: Vec<Candidate>,
    selected: HashSet<String>,
    days: u16,
    archived_only: bool,
    cutoff: i64,
    limit: usize,
}

#[derive(Clone)]
enum Control {
    Refresh,
    Review,
    Worktrees,
    Archive,
    Close,
    More,
    Delete,
    DaysMenu,
    IncludeMenu,
    Days(u16),
    Include(bool),
    Toggle(String),
}
#[derive(Clone, Copy, PartialEq)]
enum Menu {
    Days,
    Include,
}
enum Measurement {
    Progress(Usage, usize),
    Done(Result<(), String>, bool),
}

pub struct StoragePage {
    settings: Entity<SettingsPage>,
    usage: Option<Usage>,
    completed: usize,
    started: bool,
    scanning: bool,
    refresh_after_scan: bool,
    measured: bool,
    error: bool,
    cleanup_supported: bool,
    loading_review: bool,
    review_error: bool,
    review: Option<Review>,
    menu: Option<Menu>,
    confirmation: Option<Entity<Confirmation>>,
    toaster: Entity<Toaster>,
    controls: HashMap<String, FocusHandle>,
    dialog_order: Vec<FocusHandle>,
    return_focus: Option<FocusHandle>,
    focused: Option<FocusHandle>,
}

impl StoragePage {
    pub fn new(settings: Entity<SettingsPage>, cx: &mut Context<Self>) -> Self {
        if !cx.has_global::<CleanupState>() {
            cx.set_global(CleanupState::default());
        }
        cx.observe_global::<CleanupState>(|this, cx| {
            if !cx.global::<CleanupState>().deleting {
                if this.scanning {
                    this.refresh_after_scan = true;
                } else {
                    this.refresh(cx);
                }
            }
            cx.notify();
        })
        .detach();
        Self {
            settings,
            usage: None,
            completed: 0,
            started: false,
            scanning: false,
            refresh_after_scan: false,
            measured: false,
            error: false,
            cleanup_supported: false,
            loading_review: false,
            review_error: false,
            review: None,
            menu: None,
            confirmation: None,
            toaster: cx.new(|_| Toaster::new()),
            controls: HashMap::new(),
            dialog_order: Vec::new(),
            return_focus: None,
            focused: None,
        }
    }

    fn toaster(&self, cx: &App) -> Entity<Toaster> {
        self.settings
            .read(cx)
            .toaster
            .clone()
            .unwrap_or_else(|| self.toaster.clone())
    }

    fn refresh(&mut self, cx: &mut Context<Self>) {
        if self.scanning {
            return;
        }
        self.started = true;
        self.scanning = true;
        self.completed = 0;
        self.error = false;
        let Some(AppServer(bridge)) = cx.try_global::<AppServer>() else {
            self.scanning = false;
            self.error = true;
            return;
        };
        let bridge = bridge.clone();
        let (sender, receiver) = async_channel::unbounded();
        cx.background_spawn(async move {
            let supported = storage::supports_cleanup(&bridge).await;
            let result = async {
                let info = bridge
                    .initialize("codex-gpui", env!("CARGO_PKG_VERSION"))
                    .await?;
                storage::measure(
                    &bridge,
                    &PathBuf::from(info.codex_home),
                    |usage, completed| {
                        let _ = sender.try_send(Measurement::Progress(usage, completed));
                    },
                )
                .await?;
                Ok::<_, anyhow::Error>(())
            }
            .await
            .map_err(|error| error.to_string());
            let _ = sender.send(Measurement::Done(result, supported)).await;
        })
        .detach();
        cx.spawn(async move |this, cx| {
            while let Ok(event) = receiver.recv().await {
                let done = matches!(event, Measurement::Done(..));
                if this
                    .update(cx, |this, cx| {
                        match event {
                            Measurement::Progress(usage, completed) => {
                                this.usage = Some(usage);
                                this.completed = completed;
                            }
                            Measurement::Done(result, supported) => {
                                this.error = result.is_err();
                                this.measured |= result.is_ok();
                                this.scanning = false;
                                this.cleanup_supported = supported;
                            }
                        }
                        if !this.scanning && this.refresh_after_scan {
                            this.refresh_after_scan = false;
                            this.refresh(cx);
                        }
                        cx.notify();
                    })
                    .is_err()
                {
                    break;
                }
                if done {
                    break;
                }
            }
        })
        .detach();
        cx.notify();
    }

    fn load_review(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if cx.global::<CleanupState>().deleting {
            return;
        }
        let Some(AppServer(bridge)) = cx.try_global::<AppServer>() else {
            return;
        };
        let bridge = bridge.clone();
        self.loading_review = true;
        self.review_error = false;
        self.return_focus = window.focused(cx);
        cx.spawn(async move |this, cx| {
            let result = storage::inventory(&bridge).await;
            let _ = this.update(cx, |this, cx| {
                this.loading_review = false;
                match result {
                    Ok(entries) => {
                        this.review = Some(Review {
                            entries,
                            rows: Vec::new(),
                            selected: HashSet::new(),
                            days: 90,
                            archived_only: true,
                            cutoff: 0,
                            limit: 50,
                        });
                        this.filter_review();
                    }
                    Err(_) => this.review_error = true,
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn filter_review(&mut self) {
        if let (Some(review), Some(usage)) = (&mut self.review, &self.usage) {
            review.cutoff = chrono::Utc::now().timestamp() - i64::from(review.days) * 86400;
            review.rows =
                storage::candidates(&review.entries, usage, review.cutoff, review.archived_only);
            review.selected = review
                .rows
                .iter()
                .map(|row| row.root.id().to_owned())
                .collect();
            review.limit = 50;
        }
        self.menu = None;
    }

    fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.review = None;
        self.menu = None;
        self.confirmation = None;
        if let Some(focus) = self.return_focus.take() {
            focus.focus(window, cx);
        }
        cx.notify();
    }

    fn confirm(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(review) = &self.review else { return };
        let count = review.selected.len();
        if count == 0 || cx.global::<CleanupState>().deleting {
            return;
        }
        let noun = if count == 1 { "chat" } else { "chats" };
        let confirmation = cx.new(|cx|Confirmation::new("Delete selected chats?",format!("This permanently deletes {count} {noun} and their subagents. This cannot be undone."),"Delete",window,cx).destructive());
        cx.subscribe_in(&confirmation, window, |this, _, outcome, window, cx| {
            this.confirmation = None;
            if *outcome == Outcome::Confirm {
                this.delete(window, cx);
            }
            cx.notify();
        })
        .detach();
        self.confirmation = Some(confirmation);
    }

    fn delete(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(AppServer(bridge)) = cx.try_global::<AppServer>() else {
            return;
        };
        let (Some(review), Some(usage)) = (self.review.take(), self.usage.clone()) else {
            return;
        };
        let bridge = bridge.clone();
        if cx.global::<CleanupState>().deleting {
            return;
        }
        cx.update_global::<CleanupState, _>(|state, _| state.deleting = true);
        self.close(window, cx);
        let toaster = self.toaster(cx);
        let progress = toaster.update(cx, |toaster, cx| {
            toaster.push_persistent(Toast::info("Deleting chats"), cx)
        });
        cx.spawn(async move |_this, cx| {
            let result = storage::delete_selected(
                &bridge,
                &review.rows,
                &review.selected,
                &usage,
                review.cutoff,
                review.archived_only,
            )
            .await;
            let toast = match result {
                Ok(result) => {
                    let message = format!(
                        "Chats deleted: {}. Changed chats skipped: {}. Incomplete deletions: {}.",
                        result.deleted, result.skipped, result.failed
                    );
                    if result.failed > 0 {
                        Toast::danger(message)
                    } else {
                        Toast::success(message)
                    }
                }
                Err(_) => {
                    Toast::danger("Couldn't recheck the chats. Nothing was deleted. Try again.")
                }
            };
            cx.update(|cx| {
                toaster.update(cx, |toaster, cx| {
                    toaster.dismiss(progress, cx);
                    toaster.push(toast, cx);
                });
                cx.update_global::<CleanupState, _>(|state, _| state.deleting = false);
            });
        })
        .detach();
    }

    fn act(&mut self, action: Control, window: &mut Window, cx: &mut Context<Self>) {
        match action {
            Control::Refresh => self.refresh(cx),
            Control::Review => self.load_review(window, cx),
            Control::Archive => self.settings.update(cx, |settings, cx| {
                settings.section = "Archived chats";
                cx.notify();
            }),
            Control::Worktrees => self.settings.update(cx, |settings, cx| {
                settings.section = "Worktrees";
                cx.notify();
            }),
            Control::Close => self.close(window, cx),
            Control::Delete => self.confirm(window, cx),
            Control::More => {
                if let Some(review) = &mut self.review {
                    review.limit += 50;
                }
            }
            Control::DaysMenu => self.menu = (self.menu != Some(Menu::Days)).then_some(Menu::Days),
            Control::IncludeMenu => {
                self.menu = (self.menu != Some(Menu::Include)).then_some(Menu::Include)
            }
            Control::Days(days) => {
                if let Some(review) = &mut self.review {
                    review.days = days;
                }
                self.filter_review();
            }
            Control::Include(archived) => {
                if let Some(review) = &mut self.review {
                    review.archived_only = archived;
                }
                self.filter_review();
            }
            Control::Toggle(id) => {
                if let Some(review) = &mut self.review
                    && !review.selected.remove(&id)
                {
                    review.selected.insert(id);
                }
            }
        }
        cx.notify();
    }

    fn control(
        &mut self,
        id: impl Into<String>,
        label: impl Into<SharedString>,
        enabled: bool,
        action: Control,
        cx: &mut Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        let t = cx.theme().clone();
        let id = id.into();
        let label = label.into();
        let focus = self
            .controls
            .entry(id.clone())
            .or_insert_with(|| cx.focus_handle().tab_stop(true))
            .clone();
        if self.review.is_some() && id.starts_with("review-") && enabled {
            self.dialog_order.push(focus.clone());
        }
        let key_action = action.clone();
        let checkbox = matches!(action, Control::Toggle(_));
        let destructive = matches!(action, Control::Delete);
        let dropdown = matches!(action, Control::DaysMenu | Control::IncludeMenu);
        let checked = match &action {
            Control::Toggle(id) => self
                .review
                .as_ref()
                .is_some_and(|review| review.selected.contains(id)),
            _ => false,
        };
        div()
            .id(SharedString::from(id))
            .role(if checkbox {
                gpui::Role::CheckBox
            } else {
                gpui::Role::Button
            })
            .aria_label(label.clone())
            .aria_disabled(!enabled)
            .when(checkbox, |d| {
                d.aria_toggled(if checked {
                    gpui::Toggled::True
                } else {
                    gpui::Toggled::False
                })
            })
            .track_focus(&focus)
            .h(px(28.))
            .px(px(9.))
            .relative()
            .flex()
            .items_center()
            .justify_center()
            .child(super::on_card(
                12.5,
                shapes::Corner::Squircle,
                if destructive {
                    t.text_error
                } else {
                    t.text_alpha_5
                },
                None,
            ))
            .when(checkbox, |d| {
                d.size(px(16.)).p_0().rounded(px(4.)).bg(if checked {
                    t.button_primary_solid
                } else {
                    t.background_panel
                })
            })
            .when(!enabled, |d| d.opacity(0.4))
            .when(enabled && self.focused.as_ref() == Some(&focus), |d| {
                d.child(shapes::decoration(shapes::Decoration {
                    radius: gpui::Corners::all(px(12.5)),
                    border: Some((px(2.), t.focus_ring)),
                    ..Default::default()
                }))
            })
            .child(if checkbox {
                div()
                    .size(px(14.))
                    .when(checked, |d| {
                        d.child(icon("menu-check", 14., t.text_inverse))
                    })
                    .into_any_element()
            } else {
                text(
                    label,
                    13.,
                    18.,
                    430.,
                    if destructive { t.text_inverse } else { t.text },
                )
                .into_any_element()
            })
            .when(dropdown, |d| {
                d.gap(px(4.))
                    .child(icon("chevron-down", 14., t.text_tertiary))
            })
            .when(enabled, |d| {
                d.on_click(cx.listener(move |this, _, window, cx| {
                    this.act(action.clone(), window, cx);
                    cx.stop_propagation();
                }))
                .on_key_down(cx.listener(
                    move |this, event: &KeyDownEvent, window, cx| {
                        if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                            this.act(key_action.clone(), window, cx);
                            cx.stop_propagation();
                        }
                    },
                ))
            })
    }

    fn review_key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if self.confirmation.is_some() {
            return;
        }
        match event.keystroke.key.as_str() {
            "escape" => {
                if self.menu.take().is_none() {
                    self.close(window, cx);
                }
            }
            "tab" => {
                let len = self.dialog_order.len();
                if len > 0 {
                    let current = self
                        .dialog_order
                        .iter()
                        .position(|focus| focus.is_focused(window));
                    let next = if event.keystroke.modifiers.shift {
                        current.unwrap_or(0).checked_sub(1).unwrap_or(len - 1)
                    } else {
                        current.map_or(0, |index| (index + 1) % len)
                    };
                    self.dialog_order[next].focus(window, cx);
                }
            }
            _ => return,
        }
        cx.stop_propagation();
        cx.notify();
    }

    fn review_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) -> gpui::AnyElement {
        let t = cx.theme().clone();
        self.dialog_order.clear();
        let review = self.review.as_ref().expect("open review");
        let days = review.days;
        let archived = review.archived_only;
        let count = review.selected.len();
        let subagents: usize = review
            .rows
            .iter()
            .filter(|row| review.selected.contains(row.root.id()))
            .map(|row| row.descendants.len())
            .sum();
        let rows: Vec<_> = review
            .rows
            .iter()
            .take(review.limit)
            .map(|row| {
                (
                    row.root.id().to_owned(),
                    row.root.title().to_owned(),
                    row.bytes,
                    review.selected.contains(row.root.id()),
                )
            })
            .collect();
        let more = review.limit < review.rows.len();
        let cancel = self.control("review-cancel", "Cancel", true, Control::Close, cx);
        let mut days_button = self.control(
            "review-days",
            format!("Over {days} days ago"),
            true,
            Control::DaysMenu,
            cx,
        );
        let mut include_button = self.control(
            "review-include",
            if archived {
                "Archived chats"
            } else {
                "All chats"
            },
            true,
            Control::IncludeMenu,
            cx,
        );
        if let Some(menu) = self.menu {
            let choices: Vec<_> = match menu {
                Menu::Days => [30, 90, 180, 365]
                    .into_iter()
                    .map(|days| {
                        self.control(
                            format!("review-days-{days}"),
                            format!("Over {days} days ago"),
                            true,
                            Control::Days(days),
                            cx,
                        )
                        .into_any_element()
                    })
                    .collect(),
                Menu::Include => [true, false]
                    .into_iter()
                    .map(|archived| {
                        self.control(
                            format!("review-include-{archived}"),
                            if archived {
                                "Archived chats"
                            } else {
                                "All chats"
                            },
                            true,
                            Control::Include(archived),
                            cx,
                        )
                        .into_any_element()
                    })
                    .collect(),
            };
            let popup = div()
                .absolute()
                .top_full()
                .right_0()
                .mt(px(4.))
                .w(px(220.))
                .p(px(4.))
                .rounded(px(12.))
                .bg(t.menu_background)
                .occlude()
                .flex()
                .flex_col()
                .gap(px(4.))
                .children(choices);
            match menu {
                Menu::Days => days_button = days_button.child(popup),
                Menu::Include => include_button = include_button.child(popup),
            }
        }
        let body = div()
            .flex()
            .flex_col()
            .child(super::settings_row(
                "Last updated",
                None,
                days_button.into_any_element(),
                &t,
            ))
            .child(super::settings_row(
                "Include",
                None,
                include_button.into_any_element(),
                &t,
            ));
        let mut list = div()
            .id("storage-review-chats")
            .min_h_0()
            .flex_1()
            .overflow_y_scroll()
            .flex()
            .flex_col();
        if rows.is_empty() {
            list = list.child(text(
                "No chats match these filters",
                14.,
                20.,
                430.,
                t.text_secondary,
            ));
        }
        for (id, title, bytes, _) in rows {
            let control = self.control(
                format!("review-chat-{id}"),
                format!("Select {title}"),
                true,
                Control::Toggle(id),
                cx,
            );
            list = list.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .py(px(10.))
                    .child(div().flex_1().child(text(title, 13., 18., 500., t.text)))
                    .child(text(format_bytes(bytes), 14., 20., 430., t.text_secondary))
                    .child(control),
            );
        }
        if more {
            list =
                list.child(self.control("review-more", "Show more chats", true, Control::More, cx));
        }
        let delete = self.control(
            "review-delete",
            "Delete selected",
            count > 0,
            Control::Delete,
            cx,
        );
        if window
            .focused(cx)
            .is_none_or(|focus| !self.dialog_order.iter().any(|handle| handle == &focus))
            && self.confirmation.is_none()
            && let Some(focus) = self.controls.get("review-cancel")
        {
            focus.focus(window, cx);
        }
        div().absolute().inset_0().occlude().flex().items_center().justify_center().bg(t.dialog_backdrop)
            .on_mouse_down(MouseButton::Left,cx.listener(|this,_,window,cx|this.close(window,cx)))
            .child(div().id("storage-review-dialog").role(gpui::Role::Dialog).aria_label("Review old chats")
                .w(px(640.)).max_w(relative(0.95)).max_h(relative(0.9)).p(px(24.)).flex().flex_col().gap(px(20.)).rounded(px(24.)).bg(t.menu_background)
                .on_mouse_down(MouseButton::Left,|_,_,cx|cx.stop_propagation())
                .capture_key_down(cx.listener(Self::review_key))
                .child(div().flex().justify_between().child(text("Review old chats",20.,26.,500.,t.text)).child(self.control("review-close","×",true,Control::Close,cx)))
                .child(text("Sizes include subagents. Deleting a chat and its subagents is permanent.",14.,20.,430.,t.text_secondary).whitespace_normal())
                .child(body).child(list)
                .child(text(format!("Selected chats: {count} · Subagents: {subagents}"),14.,20.,430.,t.text_secondary))
                .child(div().flex().justify_end().gap(px(8.)).child(cancel).child(delete)))
            .into_any_element()
    }
}

pub fn format_bytes(bytes: u64) -> String {
    if bytes < 1024 {
        let number = if bytes >= 1000 {
            format!("{},{:03}", bytes / 1000, bytes % 1000)
        } else {
            bytes.to_string()
        };
        return format!("{number} {}", if bytes == 1 { "byte" } else { "bytes" });
    }
    let units = ["kB", "MB", "GB", "TB"];
    let mut size = bytes as f64 / 1024.;
    let mut unit = 0;
    while size >= 1024. && unit < units.len() - 1 {
        size /= 1024.;
        unit += 1;
    }
    let mut number = format!("{:.1}", (size * 10.).round() / 10.);
    let mut separator = number.find('.').unwrap_or(number.len());
    while separator > 3 {
        separator -= 3;
        number.insert(separator, ',');
    }
    format!(
        "{} {}",
        number.strip_suffix(".0").unwrap_or(&number),
        units[unit]
    )
}

impl Render for StoragePage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.focused = window.focused(cx);
        if !self.started {
            self.refresh(cx);
        }
        let t = cx.theme().clone();
        let colors = if t.appearance == Appearance::Dark {
            [
                rgb(0xffd240).into(),
                rgb(0xad7bf9).into(),
                rgb(0xfb6a22).into(),
                rgb(0xff6764).into(),
                rgb(0x40c977).into(),
                t.text_tertiary,
            ]
        } else {
            [
                rgb(0xffc300).into(),
                rgb(0x924ff7).into(),
                rgb(0xe25507).into(),
                rgb(0xe02e2a).into(),
                rgb(0x00a240).into(),
                t.text_tertiary,
            ]
        };
        let title = self.usage.as_ref().map_or_else(
            || "Codex home".to_owned(),
            |usage| {
                format!(
                    "{} {}",
                    format_bytes(usage.total()),
                    if !self.measured && self.completed < 6 {
                        "measured so far"
                    } else {
                        "used by Codex"
                    }
                )
            },
        );
        let refresh = self.control(
            "storage-refresh",
            if self.scanning {
                "Updating…"
            } else {
                "Refresh"
            },
            !self.scanning,
            Control::Refresh,
            cx,
        );
        let mut chart = div().flex().flex_col().gap(px(12.)).child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .child(text(title, 14., 20., 600., t.text))
                .child(refresh),
        );
        if let Some(usage) = &self.usage {
            let mut bar = div()
                .h(px(24.))
                .w_full()
                .flex()
                .overflow_hidden()
                .rounded(px(8.))
                .bg(t.border);
            let mut labels = div().flex().flex_wrap().gap_x(px(16.)).gap_y(px(6.));
            for (index, &bytes) in usage.bytes.iter().enumerate() {
                if usage.total() > 0 {
                    bar = bar.child(
                        div()
                            .h_full()
                            .w(relative(bytes as f32 / usage.total() as f32))
                            .relative()
                            .child(shapes::decoration(shapes::Decoration {
                                fill: Some(colors[index]),
                                ..Default::default()
                            })),
                    );
                }
                let done = self.completed >= 6
                    || (index < 4 && self.completed >= 4)
                    || (index == 5 && self.completed >= 5);
                let size = format_bytes(bytes);
                let label = if done {
                    size
                } else {
                    format!(
                        "{size} · {}",
                        if self.error {
                            "Incomplete"
                        } else {
                            "Updating…"
                        }
                    )
                };
                labels = labels.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(4.))
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(8.))
                                .child(div().size(px(8.)).rounded_full().bg(colors[index]))
                                .child(text(
                                    storage::CATEGORIES[index],
                                    14.,
                                    20.,
                                    430.,
                                    t.text_secondary,
                                )),
                        )
                        .child(text(label, 12., 16., 430., t.text_secondary)),
                );
            }
            chart = chart
                .child(crate::rounded_clip(px(8.), shapes::Corner::Round, bar))
                .child(labels)
                .child(
                    text(
                        usage.home.to_string_lossy().into_owned(),
                        12.,
                        16.,
                        430.,
                        t.text_secondary,
                    )
                    .whitespace_normal(),
                );
        } else if self.scanning {
            chart = chart.child(div().h(px(24.)).rounded(px(8.)).bg(t.border));
        }
        if self.error {
            chart = chart.child(text(
                "Couldn't measure storage. Try refreshing.",
                14.,
                20.,
                430.,
                t.text_secondary,
            ));
        }
        let review = self.control(
            "storage-review",
            if self.loading_review {
                "Updating…"
            } else {
                "Review"
            },
            self.cleanup_supported
                && self.usage.is_some()
                && self.completed >= 4
                && !self.scanning
                && !cx.global::<CleanupState>().deleting
                && !self.loading_review,
            Control::Review,
            cx,
        );
        let cleanup = super::settings_row(
            "Clean up old chats",
            Some(if self.cleanup_supported {
                "Review chats and their subagents before permanently deleting them"
            } else {
                "Update Codex on this device to clean up chats safely"
            }),
            review.into_any_element(),
            &t,
        );
        let worktrees = self.control("storage-worktrees", "Manage", true, Control::Worktrees, cx);
        let archive = self.control("storage-archive", "Manage", true, Control::Archive, cx);
        let links = div()
            .flex()
            .flex_col()
            .child(super::settings_row(
                "Manage worktrees",
                Some("Review working copies and choose how many old worktrees to keep"),
                worktrees.into_any_element(),
                &t,
            ))
            .child(super::settings_row(
                "Archived chats",
                Some("Restore or delete individual archived chats"),
                archive.into_any_element(),
                &t,
            ));
        let card = |content: gpui::Div| {
            div()
                .relative()
                .p(px(1.))
                .child(super::on_card(
                    20.,
                    shapes::Corner::Squircle,
                    t.background_panel,
                    Some(t.border),
                ))
                .child(content)
        };
        let mut content = div()
            .w_full()
            .max_w(px(768.))
            .p(px(20.))
            .flex()
            .flex_col()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .pb(px(12.))
                    .child(text("This device", 18., 27., 500., t.text))
                    .child(text(
                        "Manage chats and worktrees stored on this computer",
                        14.,
                        18.,
                        430.,
                        t.text_secondary,
                    )),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(16.))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(40.))
                            .child(chart)
                            .child(card(cleanup)),
                    )
                    .child(card(links)),
            );
        if self.review_error {
            content = content.child(text(
                "Couldn't load all chats. Try reviewing again.",
                14.,
                20.,
                430.,
                t.text_secondary,
            ));
        }
        div()
            .size_full()
            .relative()
            .flex()
            .flex_col()
            .child(div().h(px(46.)).flex_none())
            .child(
                div()
                    .id("storage-page")
                    .flex_1()
                    .overflow_y_scroll()
                    .flex()
                    .flex_col()
                    .items_center()
                    .child(
                        div()
                            .w_full()
                            .max_w(px(768.))
                            .px(px(20.))
                            .pt(px(28.))
                            .pb(px(12.))
                            .child(text("Storage", 28., 33.6, 500., t.text)),
                    )
                    .child(content),
            )
            .when(self.review.is_some(), |d| {
                let viewport = window.viewport_size();
                let dialog = match self.confirmation.clone() {
                    Some(confirmation) => confirmation.into_any_element(),
                    None => self.review_dialog(window, cx),
                };
                let overlay = div()
                    .relative()
                    .w(viewport.width)
                    .h(viewport.height)
                    .child(dialog);
                d.child(deferred(
                    anchored().position(point(px(0.), px(0.))).child(overlay),
                ))
            })
            .when(self.settings.read(cx).toaster.is_none(), |d| {
                d.child(self.toaster.clone())
            })
    }
}

#[cfg(test)]
#[path = "storage_ui_tests.rs"]
mod tests;
