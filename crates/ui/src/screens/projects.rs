//! Explore ▸ `Projects` (`/projects`): the original's `ProjectsIndexPage` over the stored local
//! projects (`local-projects`), with search, the `Name` / `Updated` sort, `Create`, `Edit
//! project` and `Start new chat in project`.

use crate::composer::Composer;
use crate::screens::pull_requests::compact_age;
use crate::screens::sidebar::Project;
use crate::screens::thread::view::Clock;
use crate::{EmptySidebar, Shell, icon::icon, text};
use gpui::{
    AnyElement, App, AppContext as _, Context, Entity, EventEmitter, InteractiveElement,
    IntoElement, KeyDownEvent, ParentElement, Render, SharedString, StatefulInteractiveElement,
    Styled, Window, div, prelude::FluentBuilder as _, px,
};
use theme::{ActiveTheme as _, Theme};

#[derive(Clone, Debug, Default, PartialEq)]
pub enum Load {
    #[default]
    Loading,
    Ready(Vec<Project>),
    Failed(String),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SortKey {
    Name,
    Updated,
}

/// What the page asks the app to do.
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    Retry,
    Create,
    /// `Start new chat in project` (or a double click on the row): the project's root.
    Open(SharedString),
    /// Actions ▸ `Edit project`.
    Edit {
        id: SharedString,
        name: SharedString,
    },
}

pub struct Page {
    pub load: Load,
    search: Entity<Composer>,
    sort: SortKey,
    descending: bool,
    /// The project whose `Project actions` menu is open.
    menu: Option<SharedString>,
    clock: Clock,
}

impl EventEmitter<Event> for Page {}

impl Page {
    pub fn new(clock: Clock, cx: &mut Context<Self>) -> Self {
        let search =
            cx.new(|cx| Composer::embedded_field("Search projects", "", px(13.), px(20.), cx));
        cx.observe(&search, |_, _, cx| cx.notify()).detach();
        Self {
            load: Load::Loading,
            search,
            sort: SortKey::Updated,
            descending: true,
            menu: None,
            clock,
        }
    }

    /// The rows the table shows: name matches of the query, in the header's sort order.
    pub fn rows(&self, cx: &App) -> Vec<&Project> {
        let Load::Ready(projects) = &self.load else {
            return Vec::new();
        };
        rows(
            projects,
            self.search.read(cx).field_text(),
            self.sort,
            self.descending,
        )
    }

    /// A header click: the active column flips its direction, another column takes its own
    /// default (`Updated` newest first, `Name` A to Z), as the original's `Gi`.
    pub fn sort_by(&mut self, key: SortKey, cx: &mut Context<Self>) {
        if self.sort == key {
            self.descending = !self.descending;
        } else {
            self.sort = key;
            self.descending = key == SortKey::Updated;
        }
        cx.notify();
    }

    fn state(&self, title: &str, description: Option<&str>, t: &Theme) -> gpui::Div {
        div()
            .flex()
            .flex_col()
            .items_center()
            .gap(px(7.))
            .pt(px(44.))
            .child(text(title.to_owned(), 16., 24., 500., t.text))
            .children(
                description.map(|value| text(value.to_owned(), 13., 18.57, 430., t.text_secondary)),
            )
    }

    fn header_cell(
        &self,
        key: SortKey,
        label: &'static str,
        t: &Theme,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let active = self.sort == key;
        div()
            .id(label)
            .role(gpui::Role::Button)
            .aria_label(label)
            .flex()
            .items_center()
            .gap(px(4.))
            .cursor_pointer()
            .child(text(label, 12., 18., 430., t.text_tertiary))
            .when(active, |d| {
                let arrow = icon("projects-sort-arrow", 14., t.text_tertiary);
                d.child(if self.descending {
                    arrow.with_transformation(gpui::Transformation::rotate(gpui::radians(
                        std::f32::consts::PI,
                    )))
                } else {
                    arrow
                })
            })
            .on_click(cx.listener(move |this, _, _, cx| this.sort_by(key, cx)))
    }

    fn icon_button(
        id: SharedString,
        name: &'static str,
        label: &'static str,
        t: &Theme,
    ) -> gpui::Stateful<gpui::Div> {
        div()
            .id(id)
            .role(gpui::Role::Button)
            .relative()
            .aria_label(label)
            .size(px(32.))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(10.))
            .cursor_pointer()
            .hover(|s| s.bg(t.ghost_hover))
            .child(icon(name, 16., t.text_secondary))
    }

    fn row(&self, project: &Project, t: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let id = project.id.clone();
        let updated = project.updated_at.map_or_else(
            || "-".to_owned(),
            |ms| compact_age(ms.div_euclid(1000), self.clock),
        );
        let open = project.root.clone();
        let edit = (project.id.clone(), project.label.clone());
        let menu_open = self.menu.as_ref() == Some(&project.id);
        div()
            .id(SharedString::from(format!("project-{id}")))
            .relative()
            .min_h(px(71.))
            .py(px(8.))
            .border_b_1()
            .border_color(t.border)
            .flex()
            .items_center()
            .gap(px(16.))
            .group("project-row")
            .child(
                div()
                    .absolute()
                    .top_0()
                    .bottom_0()
                    .left(px(-12.))
                    .right(px(-12.))
                    .rounded(px(12.))
                    .when(menu_open, |d| d.bg(t.ghost_hover))
                    .group_hover("project-row", |s| s.bg(t.ghost_hover)),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .child(
                        div()
                            .size(px(28.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(icon("folder", 16., t.text_secondary)),
                    )
                    .child(text(project.label.clone(), 14., 21., 430., t.text).truncate()),
            )
            .child(
                div()
                    .w(px(64.))
                    .child(text(updated, 14., 21., 430., t.text_secondary)),
            )
            .child(
                div()
                    .w(px(128.))
                    .flex()
                    .justify_end()
                    .gap(px(4.))
                    .child(
                        Self::icon_button(
                            format!("project-actions-{id}").into(),
                            "diff-more",
                            "Project actions",
                            t,
                        )
                        .on_click(cx.listener({
                            let id = id.clone();
                            move |this, _, _, cx| {
                                this.menu = if this.menu.as_ref() == Some(&id) {
                                    None
                                } else {
                                    Some(id.clone())
                                };
                                cx.notify();
                            }
                        }))
                        .when(menu_open, |d| d.child(self.actions_menu(edit, t, cx))),
                    )
                    .child(
                        Self::icon_button(
                            format!("project-new-chat-{id}").into(),
                            "new-chat",
                            "Start new chat in project",
                            t,
                        )
                        .when_some(open.clone(), |d, root| {
                            d.on_click(
                                cx.listener(move |_, _, _, cx| cx.emit(Event::Open(root.clone()))),
                            )
                        })
                        .when(open.is_none(), |d| d.opacity(0.4)),
                    ),
            )
            .when_some(open, |d, root| {
                d.on_click(cx.listener(move |_, event: &gpui::ClickEvent, _, cx| {
                    if event.click_count() == 2 {
                        cx.emit(Event::Open(root.clone()));
                    }
                }))
            })
    }

    /// `Project actions`: `Edit project` (pin, archive and remove need stores the port lacks).
    fn actions_menu(
        &self,
        (id, name): (SharedString, SharedString),
        t: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let hover = t.menu_item_hover;
        gpui::deferred(
            div().absolute().top_0().right_0().child(
                gpui::anchored()
                    .anchor(gpui::Anchor::TopRight)
                    .offset(gpui::point(px(-1.), px(34.)))
                    .child(
                        crate::composer::popover(t, 160., |d| {
                            d.child(
                                div()
                                    .id("project-edit")
                                    .role(gpui::Role::MenuItem)
                                    .aria_label("Edit project")
                                    .flex()
                                    .items_center()
                                    .gap(px(8.))
                                    .px(px(8.))
                                    .h(px(29.))
                                    .rounded(px(12.))
                                    .hover(move |s| s.bg(hover))
                                    .child(icon("settings-general", 16., t.text))
                                    .child(text("Edit project", 13., 18., 430., t.text))
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.menu = None;
                                        cx.emit(Event::Edit {
                                            id: id.clone(),
                                            name: name.clone(),
                                        });
                                        cx.notify();
                                    })),
                            )
                        })
                        .on_mouse_down_out(cx.listener(
                            |this, _, _, cx| {
                                this.menu = None;
                                cx.notify();
                            },
                        )),
                    ),
            ),
        )
        .with_priority(1)
        .into_any_element()
    }

    fn create(t: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("projects-create")
            .role(gpui::Role::Button)
            .aria_label("Create")
            .relative()
            .h(px(32.))
            .px(px(16.))
            .flex()
            .items_center()
            .gap(px(6.))
            .cursor_pointer()
            .focusable()
            .tab_index(0)
            .child(shapes::decoration(shapes::Decoration {
                radius: gpui::Corners::all(px(16.)),
                fill: Some(t.button_primary_solid),
                ..Default::default()
            }))
            .child(icon("plus", 14., t.text_inverse))
            .child(text("Create", 14., 20., 430., t.text_inverse))
            .on_click(cx.listener(|_, _, _, cx| cx.emit(Event::Create)))
            .on_key_down(cx.listener(|_, event: &KeyDownEvent, _, cx| {
                if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                    cx.emit(Event::Create);
                    cx.stop_propagation();
                }
            }))
    }

    fn search_box(&self, t: &Theme) -> impl IntoElement {
        div()
            .id("projects-search")
            .relative()
            .w(px(240.))
            .h(px(32.))
            .px(px(12.))
            .flex()
            .items_center()
            .gap(px(8.))
            .child(shapes::decoration(shapes::Decoration {
                radius: gpui::Corners::all(px(16.)),
                border: Some((px(1.), t.border)),
                ..Default::default()
            }))
            .child(icon("search", 16., t.text_secondary))
            .child(div().flex_1().min_w_0().child(self.search.clone()))
    }
}

impl Render for Page {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.theme().clone();
        let body = match &self.load {
            Load::Loading => self.state("Loading projects", None, &t).into_any_element(),
            Load::Failed(error) => self
                .state("Unable to load projects", Some(error), &t)
                .child(
                    crate::screens::settings::button("Retry", t.text_alpha_5, None, &t)
                        .id("projects-retry")
                        .role(gpui::Role::Button)
                        .aria_label("Retry")
                        .cursor_pointer()
                        .on_click(cx.listener(|_, _, _, cx| cx.emit(Event::Retry))),
                )
                .into_any_element(),
            Load::Ready(projects) if projects.is_empty() => self
                .state(
                    "No projects",
                    Some("Create a project to organize chats and give ChatGPT access to folders on your computer."),
                    &t,
                )
                .into_any_element(),
            Load::Ready(_) => {
                let rows: Vec<Project> = self.rows(cx).into_iter().cloned().collect();
                let mut list = div().flex().flex_col();
                for project in &rows {
                    list = list.child(self.row(project, &t, cx));
                }
                list.into_any_element()
            }
        };
        div()
            .id("projects-page")
            .size_full()
            .flex()
            .flex_col()
            .pt(px(16.))
            .pl(px(48.))
            .pr(px(58.))
            .bg(t.surface)
            .child(
                div()
                    .h(px(60.))
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(text("Projects", 24., 36., 500., t.text))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(12.))
                            .child(self.search_box(&t))
                            .child(Self::create(&t, cx)),
                    ),
            )
            .child(
                div()
                    .mt(px(1.))
                    .h(px(40.))
                    .flex()
                    .items_center()
                    .gap(px(16.))
                    .border_b_1()
                    .border_color(t.border_subtle)
                    .child(
                        div()
                            .flex_1()
                            .child(self.header_cell(SortKey::Name, "Name", &t, cx)),
                    )
                    .child(div().w(px(64.)).child(self.header_cell(
                        SortKey::Updated,
                        "Updated",
                        &t,
                        cx,
                    )))
                    .child(div().w(px(128.))),
            )
            .child(
                div()
                    .id("projects-scroll")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .pb(px(30.))
                    .child(body),
            )
    }
}

/// `rows` for the page: a case-insensitive name filter, then the sort with name and id as the
/// tie-break, like the original's `Xr` / `Zr`.
fn rows<'a>(
    projects: &'a [Project],
    query: &str,
    key: SortKey,
    descending: bool,
) -> Vec<&'a Project> {
    let query = query.trim().to_lowercase();
    let mut rows: Vec<&Project> = projects
        .iter()
        .filter(|project| query.is_empty() || project.label.to_lowercase().contains(&query))
        .collect();
    let by_name = |a: &&Project, b: &&Project| {
        a.label
            .to_lowercase()
            .cmp(&b.label.to_lowercase())
            .then_with(|| a.id.cmp(&b.id))
    };
    rows.sort_by(|a, b| {
        let order = match key {
            SortKey::Name => by_name(a, b),
            SortKey::Updated => a
                .updated_at
                .unwrap_or(0)
                .cmp(&b.updated_at.unwrap_or(0))
                .then_with(|| by_name(a, b)),
        };
        if descending { order.reverse() } else { order }
    });
    rows
}

/// `/projects`: sidebar closed, the rail's `Explore` current.
pub fn shell(page: Entity<Page>, cx: &mut App) -> Entity<Shell> {
    let sidebar = cx.new(|_| EmptySidebar).into();
    cx.new(|_| {
        let mut shell = Shell::new(sidebar, page.into());
        shell.sidebar_open = false;
        shell.selected_rail = Some("explore");
        shell.chat_chrome = false;
        shell.new_tab = false;
        shell.main_backgrounds = 1;
        shell
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project(id: &str, label: &str, updated_at: Option<i64>) -> Project {
        Project {
            id: id.to_owned().into(),
            label: label.to_owned().into(),
            root: Some(format!("/{id}").into()),
            updated_at,
            new_chat_label: label.to_owned().into(),
            worktree: false,
            missing: false,
            expanded: true,
        }
    }

    fn labels(rows: Vec<&Project>) -> Vec<&str> {
        rows.into_iter().map(|row| row.label.as_ref()).collect()
    }

    #[test]
    fn rows_filter_by_name_and_follow_the_header_sort() {
        let projects = [
            project("a", "website", Some(1_000)),
            project("b", "Proj", Some(3_000)),
            project("c", "Notes", None),
        ];
        assert_eq!(
            labels(rows(&projects, "", SortKey::Updated, true)),
            ["Proj", "website", "Notes"]
        );
        assert_eq!(
            labels(rows(&projects, "", SortKey::Name, false)),
            ["Notes", "Proj", "website"]
        );
        assert_eq!(
            labels(rows(&projects, " PRO ", SortKey::Name, false)),
            ["Proj"]
        );
    }
}
