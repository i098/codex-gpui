use crate::Root;
use gpui::Context;

pub(crate) fn thread_deleted(root: &mut Root, id: &str, cx: &mut Context<Root>) {
    root.threads.retain(|thread| thread.id != id);
    root.loaded.remove(id);
    if root
        .thread
        .as_ref()
        .is_some_and(|thread| thread.read(cx).id() == id)
    {
        root.thread = None;
    }
    root.sidebar.update(cx, |sidebar, cx| {
        sidebar.set_threads(&root.threads, cx);
    });
    if let Some(archive) = &root.archive_page {
        archive.update(cx, |archive, cx| archive.unarchived(id, cx));
    }
    cx.notify();
}

/// Performs the plan tab's file requests on the live app-server connection.
pub(crate) struct ServerPlan;

fn bridge(cx: &gpui::App) -> anyhow::Result<std::sync::Arc<bridge::Bridge>> {
    cx.try_global::<ui::screens::settings::AppServer>()
        .map(|server| server.0.clone())
        .ok_or_else(|| anyhow::anyhow!("no app-server connection"))
}

impl ui::screens::plan::PlanPort for ServerPlan {
    fn load(
        &self,
        thread_id: &str,
        key: &str,
        cx: &gpui::App,
    ) -> ui::screens::plan::Reply<(String, Option<String>)> {
        let bridge = bridge(cx);
        let (thread_id, key) = (thread_id.to_owned(), key.to_owned());
        Box::pin(async move {
            let bridge = bridge?;
            let home = bridge
                .initialize("codex-gpui", env!("CARGO_PKG_VERSION"))
                .await?
                .codex_home;
            let path = format!(
                "{}/plans/{thread_id}/{key}/PLAN.md",
                home.trim_end_matches('/')
            );
            let file = bridge
                .fs_read_file(&path)
                .await
                .ok()
                .and_then(|bytes| String::from_utf8(bytes).ok());
            Ok((path, file))
        })
    }

    fn save(&self, path: &str, content: &str, cx: &gpui::App) -> ui::screens::plan::Reply<()> {
        let bridge = bridge(cx);
        let (path, content) = (path.to_owned(), content.to_owned());
        Box::pin(async move {
            let bridge = bridge?;
            bridge
                .fs_create_directory(path.trim_end_matches("/PLAN.md"))
                .await?;
            bridge.fs_write_file(&path, content.as_bytes()).await
        })
    }
}

/// Performs the storage page's requests on the live app-server connection.
pub(crate) struct ServerStorage;

impl ui::screens::settings::storage::StoragePort for ServerStorage {
    fn supports_cleanup(&self, cx: &gpui::App) -> ui::screens::settings::storage::Reply<bool> {
        let bridge = bridge(cx);
        Box::pin(async move {
            match bridge {
                Ok(bridge) => bridge::storage::supports_cleanup(&bridge).await,
                Err(_) => false,
            }
        })
    }

    fn measure(
        &self,
        publish: Box<dyn FnMut(bridge::storage::Usage, usize) + Send>,
        cx: &gpui::App,
    ) -> ui::screens::settings::storage::Reply<anyhow::Result<()>> {
        let bridge = bridge(cx);
        Box::pin(async move {
            let bridge = bridge?;
            let info = bridge
                .initialize("codex-gpui", env!("CARGO_PKG_VERSION"))
                .await?;
            bridge::storage::measure(&bridge, std::path::Path::new(&info.codex_home), publish)
                .await?;
            Ok(())
        })
    }

    fn inventory(
        &self,
        cx: &gpui::App,
    ) -> ui::screens::settings::storage::Reply<anyhow::Result<Vec<bridge::storage::Entry>>> {
        let bridge = bridge(cx);
        Box::pin(async move { bridge::storage::inventory(&bridge?).await })
    }

    fn delete_selected(
        &self,
        reviewed: Vec<bridge::storage::Candidate>,
        selected: std::collections::HashSet<String>,
        usage: bridge::storage::Usage,
        cutoff: i64,
        archived_only: bool,
        cx: &gpui::App,
    ) -> ui::screens::settings::storage::Reply<anyhow::Result<bridge::storage::CleanupResult>> {
        let bridge = bridge(cx);
        Box::pin(async move {
            bridge::storage::delete_selected(
                &bridge?,
                &reviewed,
                &selected,
                &usage,
                cutoff,
                archived_only,
            )
            .await
        })
    }
}

pub(crate) fn install_services(cx: &mut gpui::App) {
    if !cx.has_global::<ui::screens::plan::PlanService>() {
        cx.set_global(ui::screens::plan::PlanService(std::rc::Rc::new(ServerPlan)));
        cx.set_global(ui::screens::settings::storage::StorageService(
            std::rc::Rc::new(ServerStorage),
        ));
    }
}
