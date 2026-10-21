use crate::host::Host;
use gpui::{App, Entity};
use std::sync::Arc;
use ui::screens::diff::panel::DiffPanel;

pub(crate) fn load(panel: Entity<DiffPanel>, host: Arc<dyn Host>, thread: String, cwd: String, cx: &mut App) {
    let target = panel.downgrade();
    let request = cx.background_executor().spawn(async move { host.review_repositories(&thread, &cwd) });
    cx.spawn(async move |cx| {
        match request.await {
            Ok(repositories) => { let _ = target.update(cx, |panel, cx| panel.set_repositories(repositories, cx)); }
            Err(error) => eprintln!("codex-gpui: cannot read review repositories: {error:#}"),
        }
    }).detach();
}
