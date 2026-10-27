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
