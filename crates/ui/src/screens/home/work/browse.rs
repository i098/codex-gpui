//! `library-file-picker-dialog`: `Files` ▸ `Browse all` opens `Add from Library`, where any
//! Library file (not only the recent ten) can be searched, picked and added to the chat.

use super::{LibraryFile, LibraryLoad, WorkEvent, check, local_matches, skeleton_row, toggled};
use crate::composer::Composer;
use crate::{icon::icon, text};
use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, AppContext as _, Context, Div, Entity, Focusable as _, InteractiveElement as _,
    IntoElement, MouseButton, ParentElement as _, SharedString, StatefulInteractiveElement as _,
    Styled as _, Task, Window, div, px,
};
use std::time::Duration;
use theme::Theme;

/// Files the dialog reads at once.
pub const BROWSE_LIMIT: usize = 50;

pub struct BrowseDialog {
    pub open: bool,
    search: Entity<Composer>,
    /// The query the shown files answer, and the field's text when last seen.
    pub query: String,
    typed: String,
    pub load: LibraryLoad,
    /// Picked files, in pick order.
    selected: Vec<LibraryFile>,
    grid: bool,
    debounce: Option<Task<()>>,
}

impl BrowseDialog {
    pub fn new(cx: &mut Context<super::HomeMain>) -> Self {
        let search =
            cx.new(|cx| Composer::embedded_field("Search Library", "", px(14.), px(20.), cx));
        cx.observe(&search, |main, search, cx| {
            let typed = search.read(cx).field_text().trim().to_owned();
            if typed != main.work.browse.typed {
                main.work.browse.typed = typed.clone();
                let delay = if typed.is_empty() { 0 } else { 300 };
                main.work.browse.debounce = Some(cx.spawn(async move |main, cx| {
                    cx.background_executor()
                        .timer(Duration::from_millis(delay))
                        .await;
                    main.update(cx, |main, cx| main.request_browse_files(typed, cx))
                        .ok();
                }));
            }
            cx.notify();
        })
        .detach();
        Self {
            open: false,
            search,
            query: String::new(),
            typed: String::new(),
            load: LibraryLoad::Loading,
            selected: Vec::new(),
            grid: false,
            debounce: None,
        }
    }
}

impl super::HomeMain {
    fn request_browse_files(&mut self, query: String, cx: &mut Context<Self>) {
        self.work.browse.query = query.clone();
        self.work.browse.load = LibraryLoad::Loading;
        cx.emit(WorkEvent::LibraryBrowse(query));
        cx.notify();
    }

    /// The app's answer to [`WorkEvent::LibraryBrowse`]; a stale query's answer is dropped.
    pub fn set_browse_files(
        &mut self,
        query: &str,
        files: Option<Vec<LibraryFile>>,
        cx: &mut Context<Self>,
    ) {
        if self.work.browse.query == query {
            self.work.browse.load = files.map_or(LibraryLoad::Failed, LibraryLoad::Ready);
            cx.notify();
        }
    }

    /// `Browse all`: closes the menu and opens the dialog on the recent files.
    pub(super) fn open_browse(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.close_work_menus();
        let dialog = &mut self.work.browse;
        dialog.open = true;
        dialog.selected.clear();
        dialog.typed.clear();
        dialog.debounce = None;
        let search = dialog.search.clone();
        search.update(cx, |search, cx| search.set_text("", cx));
        window.focus(&search.focus_handle(cx), cx);
        self.request_browse_files(String::new(), cx);
    }

    pub fn close_browse(&mut self, cx: &mut Context<Self>) {
        self.work.browse.open = false;
        cx.notify();
    }

    fn toggle_browse_file(&mut self, file: LibraryFile, cx: &mut Context<Self>) {
        let selected = &mut self.work.browse.selected;
        match selected.iter().position(|f| f.id == file.id) {
            Some(at) => {
                selected.remove(at);
            }
            None => selected.push(file),
        }
        cx.notify();
    }

    /// `Add to chat`: downloads every picked file that is not already attached.
    fn add_browse_files(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        for file in std::mem::take(&mut self.work.browse.selected) {
            if self.library_file_attached(&file.id, cx).is_none()
                && !self.work.files.pending.contains(&file.id)
            {
                self.work.files.pending.push(file.id.clone());
                cx.emit(WorkEvent::AttachLibraryFile(file));
            }
        }
        self.work.browse.open = false;
        window.focus(&self.composer.focus_handle(cx), cx);
        cx.notify();
    }

    /// The open dialog over the whole window.
    pub(in crate::screens::home) fn browse_dialog(
        &self,
        t: &Theme,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let dialog = &self.work.browse;
        if !dialog.open {
            return None;
        }
        let owner = cx.entity();
        let typed = dialog.search.read(cx).field_text().trim().to_owned();
        let settling = matches!(dialog.load, LibraryLoad::Loading) || typed != dialog.query;
        let files = match &dialog.load {
            LibraryLoad::Ready(files) if settling && !typed.is_empty() => {
                Some(local_matches(files, &typed))
            }
            LibraryLoad::Ready(files) => Some(files.clone()),
            _ => None,
        };
        let loading = settling && files.as_ref().is_none_or(Vec::is_empty);
        let body = if loading {
            let skeleton = div().flex().flex_col();
            let skeleton = skeleton.child(status("Loading recent files…", t));
            skeleton.children((0..7).map(|i| skeleton_row(i, t)))
        } else if dialog.load == LibraryLoad::Failed {
            let retry_owner = owner.clone();
            let query = dialog.query.clone();
            div()
                .flex()
                .flex_col()
                .items_center()
                .gap(px(8.))
                .py(px(48.))
                .child(text("Unable to load Library files", 14., 20., 500., t.text))
                .child(text(
                    "Try again in a moment",
                    14.,
                    20.,
                    400.,
                    t.text_tertiary,
                ))
                .child(button("library-browse-retry", "Retry", false, t).on_click(
                    move |_, _, cx| {
                        retry_owner
                            .update(cx, |main, cx| main.request_browse_files(query.clone(), cx))
                    },
                ))
        } else {
            let files = files.unwrap_or_default();
            if files.is_empty() {
                div().child(status(
                    if typed.is_empty() {
                        "No recent files yet"
                    } else {
                        "No matching files"
                    },
                    t,
                ))
            } else {
                let tiles = files.into_iter().enumerate().map(|(i, file)| {
                    let picked = dialog.selected.iter().any(|f| f.id == file.id);
                    let enabled = file.attachable();
                    let owner = owner.clone();
                    let name = SharedString::from(file.name.clone());
                    let hover = t.menu_item_hover;
                    div()
                        .id(("library-browse-file", i))
                        .debug_selector(move || format!("library-browse-file-{i}"))
                        .role(gpui::Role::CheckBox)
                        .aria_label(name.clone())
                        .aria_toggled(toggled(picked))
                        .aria_disabled(!enabled)
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .px(px(8.))
                        .py(px(6.))
                        .rounded(px(12.))
                        .when(dialog.grid, |d| {
                            d.w(px(180.)).border_1().border_color(t.border)
                        })
                        .when(picked, |d| d.bg(t.menu_item_hover))
                        .when(!enabled, |d| d.opacity(0.5))
                        .when(enabled, |d| {
                            d.hover(move |s| s.bg(hover)).on_click(move |_, _, cx| {
                                owner.update(cx, |main, cx| {
                                    main.toggle_browse_file(file.clone(), cx)
                                })
                            })
                        })
                        .child(check(picked, t))
                        .child(icon("context-file-document", 16., t.text).opacity(0.75))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .overflow_hidden()
                                .child(text(name, 14., 20., 400., t.text)),
                        )
                });
                if dialog.grid {
                    div().flex().flex_wrap().gap(px(8.)).children(tiles)
                } else {
                    div().flex().flex_col().children(tiles)
                }
            }
        };

        let view = |id: &'static str, label: &'static str, glyph: &'static str, grid: bool| {
            let owner = owner.clone();
            let active = dialog.grid == grid;
            div()
                .id(id)
                .role(gpui::Role::Button)
                .aria_label(label)
                .aria_toggled(toggled(active))
                .size(px(28.))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(8.))
                .when(active, |d| d.bg(t.ghost_hover))
                .child(icon(glyph, 16., t.text))
                .on_click(move |_, _, cx| {
                    owner.update(cx, |main, cx| {
                        main.work.browse.grid = grid;
                        cx.notify();
                    })
                })
        };
        let upload_owner = owner.clone();
        let upload = button("library-browse-upload", "Upload files", false, t).on_click(
            move |_, window, cx| {
                upload_owner.update(cx, |main, cx| {
                    main.close_browse(cx);
                    window.focus(&main.composer.focus_handle(cx), cx);
                    main.composer.update(cx, |c, cx| c.add_files(cx));
                })
            },
        );
        let count = dialog.selected.len();
        let cancel_owner = owner.clone();
        let add_owner = owner.clone();
        let close_owner = owner.clone();
        let key_owner = owner.clone();
        let content = div()
            .id("library-file-picker-dialog")
            .debug_selector(|| "library-file-picker-dialog".into())
            .role(gpui::Role::Dialog)
            .aria_label("Add from Library")
            .w(px(640.))
            .max_w_full()
            .h(px(520.))
            .max_h_full()
            .flex()
            .flex_col()
            .gap(px(12.))
            .p(px(20.))
            .rounded(px(24.))
            .bg(t.background_elevated_primary_opaque)
            .border_1()
            .border_color(t.border)
            .occlude()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .capture_key_down(move |event, _, cx| {
                if event.keystroke.key == "escape" {
                    cx.stop_propagation();
                    key_owner.update(cx, |main, cx| main.close_browse(cx));
                }
            })
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(text("Add from Library", 18., 26., 500., t.text))
                    .child(
                        div()
                            .id("library-browse-close")
                            .role(gpui::Role::Button)
                            .aria_label("Close")
                            .size(px(28.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded_full()
                            .hover(|s| s.bg(t.ghost_hover))
                            .child(icon("close", 16., t.text))
                            .on_click(move |_, _, cx| {
                                close_owner.update(cx, |main, cx| main.close_browse(cx))
                            }),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .items_center()
                            .gap(px(6.))
                            .px(px(10.))
                            .py(px(6.))
                            .rounded(px(12.))
                            .border_1()
                            .border_color(t.border)
                            .child(icon("shortcut-search", 14., t.text_tertiary))
                            .child(div().flex_1().min_w_0().child(dialog.search.clone())),
                    )
                    .child(view(
                        "library-view-list",
                        "List view",
                        "library-view-list",
                        false,
                    ))
                    .child(view(
                        "library-view-grid",
                        "Grid view",
                        "library-view-grid",
                        true,
                    ))
                    .child(upload),
            )
            .child(
                div()
                    .id("library-browse-files")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .child(body),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(text(
                        if count > 0 {
                            format!("{count} selected")
                        } else {
                            String::new()
                        },
                        14.,
                        20.,
                        400.,
                        t.text_secondary,
                    ))
                    .child(
                        div()
                            .flex()
                            .gap(px(8.))
                            .child(
                                button("library-browse-cancel", "Cancel", false, t).on_click(
                                    move |_, _, cx| {
                                        cancel_owner.update(cx, |main, cx| main.close_browse(cx))
                                    },
                                ),
                            )
                            .child(
                                button("library-browse-add", "Add to chat", true, t)
                                    .aria_disabled(count == 0)
                                    .when(count == 0, |d| d.opacity(0.4))
                                    .when(count > 0, |d| {
                                        d.on_click(move |_, window, cx| {
                                            add_owner.update(cx, |main, cx| {
                                                main.add_browse_files(window, cx)
                                            })
                                        })
                                    }),
                            ),
                    ),
            );
        let scrim_owner = cx.entity();
        Some(
            div()
                .absolute()
                .inset_0()
                .bg(t.dialog_backdrop)
                .flex()
                .items_center()
                .justify_center()
                .p(px(24.))
                .occlude()
                .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                    scrim_owner.update(cx, |main, cx| main.close_browse(cx))
                })
                .child(content)
                .into_any_element(),
        )
    }
}

fn status(label: &'static str, t: &Theme) -> Div {
    div()
        .px(px(8.))
        .py(px(5.))
        .child(text(label, 14., 20., 400., t.text_tertiary))
}

fn button(id: &'static str, label: &'static str, primary: bool, t: &Theme) -> gpui::Stateful<Div> {
    let (fill, hover, color) = if primary {
        (
            t.button_primary_solid,
            t.button_primary_solid_hover,
            t.surface,
        )
    } else {
        (gpui::transparent_black(), t.ghost_hover, t.text)
    };
    div()
        .id(id)
        .debug_selector(move || id.into())
        .role(gpui::Role::Button)
        .aria_label(label)
        .flex_none()
        .px(px(12.))
        .py(px(6.))
        .rounded_full()
        .bg(fill)
        .when(!primary, |d| d.border_1().border_color(t.border))
        .hover(move |s| s.bg(hover))
        .child(text(label, 14., 20., 500., color))
}
