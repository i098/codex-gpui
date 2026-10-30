use crate::*;
use gpui::Div;
use ui::OpenProjects;
use ui::screens::projects::{self, Event, Page};

pub(super) fn actions(root: Div, cx: &mut Context<Root>) -> Div {
    root.on_action(
        cx.listener(|this, _: &OpenProjects, window, cx| {
            this.navigate(Route::Projects, window, cx)
        }),
    )
}

/// The page over the host's stored projects; every load also refreshes the sidebar's list.
pub(super) fn page(root: &mut Root, window: &mut Window, cx: &mut Context<Root>) -> Entity<Shell> {
    let clock = root.clock.unwrap_or(crate::Clock {
        utc_offset: crate::utc_offset(),
        pinned_now: None,
        elapsed_since: None,
    });
    let page = cx.new(|cx| Page::new(clock, cx));
    cx.subscribe_in(&page, window, |this, _, event, window, cx| match event {
        Event::Retry => {
            this.load_projects(cx);
        }
        Event::Create => this.open_new_project(window, cx),
        Event::Open(path) => this.select_project(path.to_string(), window, cx),
        Event::Edit { id, name } => this.open_edit_project(id.clone(), name.clone(), window, cx),
    })
    .detach();
    root.projects_page = Some(page.clone());
    root.load_projects(cx);
    projects::shell(page, cx)
}
