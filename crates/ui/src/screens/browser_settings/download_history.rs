use super::super::{Layer, worktrees::card};
use super::{Back, Kind, action, frame, paragraph};
use crate::{composer::Composer, icon::icon, text};
use chrono::TimeZone as _;
use gpui::{
    AppContext as _, BorrowAppContext as _, Context, Div, Entity, EventEmitter, Global,
    InteractiveElement as _, IntoElement, ParentElement as _, Render,
    StatefulInteractiveElement as _, Styled as _, Window, div, prelude::FluentBuilder as _, px,
};
use std::path::PathBuf;
use theme::{ActiveTheme as _, Theme};

/// A completed download from the built-in browser.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Download {
    pub path: PathBuf,
    pub source: String,
    /// Unix milliseconds.
    pub time: i64,
}

/// Completed downloads in order of completion, oldest first.
#[derive(Default)]
pub struct DownloadHistory(pub Vec<Download>);

impl Global for DownloadHistory {}

pub(super) struct DownloadHistoryPage {
    search: Entity<Composer>,
    layer: Layer,
}

impl EventEmitter<Back> for DownloadHistoryPage {}

impl DownloadHistoryPage {
    pub(super) fn new(cx: &mut Context<Self>) -> Self {
        let search =
            cx.new(|cx| Composer::settings_compact_field("Search download history", "", cx));
        cx.observe(&search, |_, _, cx| cx.notify()).detach();
        cx.observe_global::<DownloadHistory>(|_, cx| cx.notify())
            .detach();
        Self {
            search,
            layer: Layer::default(),
        }
    }
}

fn row(index: usize, download: &Download, t: &Theme) -> Div {
    let name = download
        .path
        .file_name()
        .unwrap_or_default()
        .to_string_lossy();
    let time = chrono::Local
        .timestamp_millis_opt(download.time)
        .single()
        .map(|time| time.format("%b %-d, %Y, %H:%M").to_string())
        .unwrap_or_default();
    let detail = if download.source.is_empty() {
        time
    } else {
        format!("{time} · {}", download.source)
    };
    let path = download.path.clone();
    let identity = (download.path.clone(), download.time);
    div().child(
        div()
            .id(("download-history-row", index))
            .debug_selector(move || format!("download-history-row-{index}"))
            .flex()
            .items_center()
            .gap(px(8.))
            .px(px(16.))
            .py(px(12.))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .child(text(name.into_owned(), 13., 18., 500., t.text).truncate())
                    .child(text(detail, 13., 18., 430., t.text_secondary).truncate()),
            )
            .child(
                action(
                    "download-history-reveal",
                    "Show in folder",
                    Kind::Secondary,
                    false,
                    t,
                )
                .on_click(move |_, _, cx| cx.reveal_path(&path)),
            )
            .child(
                action("download-history-remove", "Remove", Kind::Danger, false, t).on_click(
                    move |_, _, cx| {
                        cx.update_global::<DownloadHistory, _>(|history, _| {
                            history
                                .0
                                .retain(|item| item.path != identity.0 || item.time != identity.1);
                        });
                    },
                ),
            ),
    )
}

impl Render for DownloadHistoryPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.theme();
        let query = self.search.read(cx).field_text().trim().to_lowercase();
        let downloads = cx
            .try_global::<DownloadHistory>()
            .map_or(&[][..], |history| history.0.as_slice());
        let rows = downloads
            .iter()
            .rev()
            .filter(|download| {
                query.is_empty()
                    || download.source.to_lowercase().contains(&query)
                    || download
                        .path
                        .file_name()
                        .is_some_and(|name| name.to_string_lossy().to_lowercase().contains(&query))
            })
            .enumerate()
            .map(|(index, download)| row(index, download, t))
            .collect::<Vec<_>>();
        let back = div()
            .id("download-history-back")
            .debug_selector(|| "download-history-back".into())
            .flex()
            .items_center()
            .gap(px(4.))
            .cursor_pointer()
            .child(icon("arrow-left", 16., t.text_tertiary).runtime())
            .child(text("Browser", 13., 18., 430., t.text_tertiary))
            .on_click(cx.listener(|_, _, _, cx| cx.emit(Back)));
        let clear = action(
            "download-history-clear",
            "Clear all",
            Kind::Secondary,
            downloads.is_empty(),
            t,
        )
        .when(!downloads.is_empty(), |button| {
            button.on_click(|_, _, cx| {
                cx.update_global::<DownloadHistory, _>(|history, _| history.0.clear());
            })
        });
        let list = if rows.is_empty() {
            let (title, detail) = if query.is_empty() {
                (
                    "No downloads yet",
                    "Files downloaded from the built-in browser will appear here",
                )
            } else {
                (
                    "No matching downloads",
                    "Try searching for a different filename or address",
                )
            };
            div()
                .child(text(title, 14., 21., 500., t.text))
                .child(paragraph(detail, t))
        } else {
            card(rows, Some(&self.layer), t)
        };
        let content = div()
            .flex()
            .flex_col()
            .gap(px(12.))
            .child(self.search.clone())
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap(px(8.))
                    .child(text("All-time history", 16., 22., 500., t.text))
                    .child(clear),
            )
            .child(list);
        frame(
            "download-history-page",
            "Download history",
            vec![back.into_any_element()],
            content,
            None,
            &self.layer,
            window,
            t,
        )
    }
}
