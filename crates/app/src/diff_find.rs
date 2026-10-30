use super::*;
use ui::screens::diff::panel::{FindInChat, FindInDiff};

impl Root {
    pub(super) fn find_in_chat(
        &mut self,
        action: &FindInChat,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(panel) = &self.diff {
            panel.update(cx, |panel, cx| panel.close_find(cx));
        }
        if let Some(thread) = &self.thread {
            thread.update(cx, |thread, cx| {
                thread.set_find_query(&action.0, window, cx)
            });
        }
    }

    pub(super) fn find_in_diff(
        &mut self,
        action: &FindInDiff,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(panel) = self.diff.clone() else {
            return;
        };
        if let Some(thread) = &self.thread {
            thread.update(cx, |thread, cx| thread.close_find(cx));
        }
        self.set_side(SidePanel::Changes, window, cx);
        panel.update(cx, |panel, cx| panel.set_find_query(&action.0, window, cx));
    }

    pub(super) fn next_search_result(&mut self, previous: bool, cx: &mut Context<Self>) {
        if self.side_visible(SidePanel::Changes)
            && let Some(panel) = &self.diff
            && panel.update(cx, |panel, cx| panel.next_find(previous, cx))
        {
            return;
        }
        if self.side_visible(SidePanel::File)
            && let Some(file) = &self.file
            && file.update(cx, |file, cx| file.next_find(previous, cx))
        {
            return;
        }
        if let Some(thread) = &self.thread {
            thread.update(cx, |thread, cx| thread.next_find(previous, cx));
        }
    }
}
