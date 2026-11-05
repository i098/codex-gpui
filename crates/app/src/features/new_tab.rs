//! PANEL-20: the side panel's `New tab` page. The strip's and the titlebar's `New tab`
//! button open it; a pick replaces the `New tab` tab with the tab it opens.

use super::command_destinations::recent_files;
use crate::*;
use gpui::Div;
use ui::screens::diff::panel::{OpenFile, OpenNewSideTab};
use ui::screens::new_tab::{NewTabPage, NewTabPick};

pub(super) fn actions(root: Div, cx: &mut Context<Root>) -> Div {
    root.on_action(
        cx.listener(|this, _: &OpenNewSideTab, window, cx| this.open_new_tab(window, cx)),
    )
}

impl Root {
    fn open_new_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(thread) = &self.thread else { return };
        let cwd = std::path::Path::new(thread.read(cx).cwd())
            .components()
            .as_path()
            .to_string_lossy()
            .into_owned();
        let (changes, recents) = (self.diff.is_some(), recent_files(&cwd, cx));
        let page = cx.new(|_| NewTabPage::new(changes, recents));
        cx.subscribe_in(&page, window, |this, _, pick: &NewTabPick, window, cx| {
            this.tabs.retain(|tab| *tab != SidePanel::NewTab);
            this.new_tab = None;
            match pick {
                NewTabPick::Changes => this.set_side(SidePanel::Changes, window, cx),
                NewTabPick::Terminal => this.show_terminal(false, window, cx),
                NewTabPick::BottomTerminal => this.show_terminal(true, window, cx),
                NewTabPick::Browser => {
                    this.open_browser_tab(None, window, cx);
                }
                NewTabPick::File(path) => window.dispatch_action(
                    Box::new(OpenFile {
                        path: path.clone(),
                        line: None,
                    }),
                    cx,
                ),
            }
            this.sync(window, cx);
        })
        .detach();
        self.new_tab = Some(page);
        self.set_side(SidePanel::NewTab, window, cx);
    }
}
