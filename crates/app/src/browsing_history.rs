//! The built-in browser's history ([`BrowsingHistory`]) for Settings ▸ Browser ▸ Browsing
//! history: loaded from `browsing-history.json` beside `state.json` and saved on every change.
use gpui::{App, BorrowAppContext as _, Entity};
use std::path::{Path, PathBuf};
use ui::browser::BrowserTab;
use ui::screens::settings::browser_settings::{BrowsingHistory, Visit};

/// Sets the global from `path` (empty when unset or unreadable) and saves each change there.
pub(crate) fn install(path: Option<PathBuf>, cx: &mut App) {
    let visits = path.as_deref().map(load).unwrap_or_default();
    cx.set_global(BrowsingHistory(visits));
    let Some(path) = path else { return };
    cx.observe_global::<BrowsingHistory>(move |cx| {
        if let Err(error) = save(&path, &cx.global::<BrowsingHistory>().0) {
            eprintln!("codex-gpui: cannot save {}: {error:#}", path.display());
        }
    })
    .detach();
}

fn load(path: &Path) -> Vec<Visit> {
    match std::fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_else(|error| {
            eprintln!("codex-gpui: cannot read {}: {error}", path.display());
            Vec::new()
        }),
        Err(_) => Vec::new(),
    }
}

fn save(path: &Path, visits: &[Visit]) -> anyhow::Result<()> {
    let directory = path.parent().unwrap_or(Path::new("."));
    std::fs::create_dir_all(directory)?;
    let file = tempfile::NamedTempFile::new_in(directory)?;
    serde_json::to_writer(&file, visits)?;
    file.persist(path)?;
    Ok(())
}

/// Records `tab`'s committed page, as Chromium history does for each navigation.
pub(crate) fn record(tab: &Entity<BrowserTab>, cx: &mut App) {
    let tab = tab.read(cx);
    let Some(url) = tab
        .url()
        .filter(|url| matches!(url.scheme(), "http" | "https"))
    else {
        return;
    };
    let (url, title) = (url.to_string(), tab.title());
    let time = chrono::Utc::now().timestamp_millis();
    if cx.has_global::<BrowsingHistory>() {
        cx.update_global::<BrowsingHistory, _>(|history, _| history.record(&url, &title, time));
    }
}

/// A title that arrives after the navigation renames its visit.
pub(crate) fn retitle(tab: &Entity<BrowserTab>, cx: &mut App) {
    let tab = tab.read(cx);
    let Some(url) = tab.url().map(ToString::to_string) else {
        return;
    };
    let title = tab.title();
    if cx.has_global::<BrowsingHistory>() {
        cx.update_global::<BrowsingHistory, _>(|history, _| history.set_title(&url, &title));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn history_survives_a_save_and_load() -> anyhow::Result<()> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("nested/browsing-history.json");
        assert_eq!(load(&path), []);
        let mut history = BrowsingHistory::default();
        history.record("https://example.com/", "Example", 1);
        history.set_title("https://example.com/", "Example Domain");
        save(&path, &history.0)?;
        assert_eq!(load(&path), history.0);
        std::fs::write(&path, "not json")?;
        assert_eq!(load(&path), []);
        Ok(())
    }
}
