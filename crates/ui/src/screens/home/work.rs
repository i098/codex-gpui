//! The Work home rail's `Files`, `Plugins` and run-location controls
//! (`work-home-files-control`, `composer-work-home-plugins-control`,
//! `composer-action-bar-run-location-dropdown`).

use super::{HomeMain, chip};
use crate::composer::{Composer, ConnectedPlugin};
use crate::{focus, icon::icon, text};
use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, App, AppContext as _, Context, Div, Entity, EventEmitter, Focusable as _,
    InteractiveElement as _, IntoElement, MouseButton, ParentElement as _, Pixels, SharedString,
    Stateful, StatefulInteractiveElement as _, Styled as _, Task, Window, anchored, deferred, div,
    point, px,
};
use std::path::PathBuf;
use std::time::Duration;
use theme::Theme;

/// The local Work limit (`47185920`): larger files and files of unknown size stay disabled.
const LOCAL_FILE_LIMIT: u64 = 45 * 1024 * 1024;
/// `Ge`: skeleton rows while the files load.
const SKELETON_ROWS: usize = 7;
const ROW_TEXT: (f32, f32, f32) = (13., 18.571_428, 430.);
/// The run-location button's 28 px square.
const RUN_BUTTON: Pixels = px(28.);

/// One Library file of the `Files` menu (`POST /files/library` item).
#[derive(Clone, Debug, PartialEq)]
pub struct LibraryFile {
    /// The Library id (`libfile_…`).
    pub id: String,
    /// The file id its content downloads by.
    pub file_id: String,
    pub name: String,
    pub size: Option<u64>,
}

impl LibraryFile {
    /// Whether a local Work task can take it (`file_size_bytes` known and at most 45 MiB).
    pub fn attachable(&self) -> bool {
        self.size.is_some_and(|size| size <= LOCAL_FILE_LIMIT)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum LibraryLoad {
    Loading,
    Failed,
    Ready(Vec<LibraryFile>),
}

/// What the app answers for the rail.
#[derive(Clone, Debug, PartialEq)]
pub enum WorkEvent {
    /// Read the recent (empty) or matching Library files; answer with
    /// [`HomeMain::set_library_files`].
    LibraryFiles(String),
    /// Download a picked file and attach it; answer with [`HomeMain::library_file_done`].
    AttachLibraryFile(LibraryFile),
    /// `Browse all plugins`: open the plugin directory.
    BrowsePlugins,
}

impl EventEmitter<WorkEvent> for HomeMain {}

pub struct FilesMenu {
    pub open: bool,
    search: Entity<Composer>,
    /// The query the shown files answer, and the field's text when last seen.
    pub query: String,
    typed: String,
    pub load: LibraryLoad,
    /// Picked files: their Library id and the composer attachment's path.
    pub attached: Vec<(String, PathBuf)>,
    /// Files still downloading.
    pub pending: Vec<String>,
    debounce: Option<Task<()>>,
}

pub struct PluginsMenu {
    pub open: bool,
    search: Entity<Composer>,
    /// `None` while the first `plugin/list` runs; a reload keeps the shown list.
    pub load: Option<Result<Vec<ConnectedPlugin>, String>>,
    /// The `Connect plugins` submenu is open.
    pub connect_open: bool,
}

pub struct WorkControls {
    pub files: FilesMenu,
    pub plugins: PluginsMenu,
    pub run_open: bool,
}

impl WorkControls {
    pub fn new(cx: &mut Context<HomeMain>) -> Self {
        let field = |placeholder, cx: &mut Context<HomeMain>| {
            cx.new(|cx| Composer::embedded_field(placeholder, "", px(13.), px(18.571_428), cx))
        };
        let files_search = field("Search Library files…", cx);
        cx.observe(&files_search, |main, search, cx| {
            let typed = search.read(cx).field_text().trim().to_owned();
            if typed != main.work.files.typed {
                main.work.files.typed = typed.clone();
                // `useDebounce(query, 300)`; an emptied field asks at once.
                let delay = if typed.is_empty() { 0 } else { 300 };
                main.work.files.debounce = Some(cx.spawn(async move |main, cx| {
                    cx.background_executor()
                        .timer(Duration::from_millis(delay))
                        .await;
                    main.update(cx, |main, cx| main.request_library_files(typed, cx))
                        .ok();
                }));
            }
            cx.notify();
        })
        .detach();
        let plugins_search = field("Search plugins…", cx);
        cx.observe(&plugins_search, |_, _, cx| cx.notify()).detach();
        Self {
            files: FilesMenu {
                open: false,
                search: files_search,
                query: String::new(),
                typed: String::new(),
                load: LibraryLoad::Loading,
                attached: Vec::new(),
                pending: Vec::new(),
                debounce: None,
            },
            plugins: PluginsMenu {
                open: false,
                search: plugins_search,
                load: None,
                connect_open: false,
            },
            run_open: false,
        }
    }
}

impl HomeMain {
    fn request_library_files(&mut self, query: String, cx: &mut Context<Self>) {
        self.work.files.query = query.clone();
        self.work.files.load = LibraryLoad::Loading;
        cx.emit(WorkEvent::LibraryFiles(query));
        cx.notify();
    }

    /// The app's answer to [`WorkEvent::LibraryFiles`]; a stale query's answer is dropped.
    pub fn set_library_files(
        &mut self,
        query: &str,
        files: Option<Vec<LibraryFile>>,
        cx: &mut Context<Self>,
    ) {
        if self.work.files.query == query {
            self.work.files.load = files.map_or(LibraryLoad::Failed, LibraryLoad::Ready);
            cx.notify();
        }
    }

    /// The app's answer to [`WorkEvent::AttachLibraryFile`]: the file's bytes, or `None`
    /// when the download failed (the app shows the toast).
    pub fn library_file_done(
        &mut self,
        file: &LibraryFile,
        bytes: Option<Vec<u8>>,
        cx: &mut Context<Self>,
    ) {
        self.work.files.pending.retain(|id| *id != file.id);
        if let Some(bytes) = bytes {
            let attached = self
                .composer
                .update(cx, |c, cx| c.attach_download(&file.name, bytes, cx));
            match attached {
                Ok(path) => self.work.files.attached.push((file.id.clone(), path)),
                Err(error) => eprintln!("codex-gpui: Library file not attached: {error:#}"),
            }
        }
        cx.notify();
    }

    /// Whether `id` is attached: picked, and its attachment still in the composer.
    fn library_file_attached(&self, id: &str, cx: &App) -> Option<PathBuf> {
        let composer = self.composer.read(cx);
        self.work
            .files
            .attached
            .iter()
            .find(|(file, path)| file == id && composer.has_attachment(path))
            .map(|(_, path)| path.clone())
    }

    /// A row's toggle (`onSelectFile`): a picked file leaves the composer, another downloads.
    fn toggle_library_file(&mut self, file: LibraryFile, cx: &mut Context<Self>) {
        if let Some(path) = self.library_file_attached(&file.id, cx) {
            self.work.files.attached.retain(|(id, _)| *id != file.id);
            self.composer
                .update(cx, |c, cx| c.remove_attachment_at(&path, cx));
        } else if file.attachable() && !self.work.files.pending.contains(&file.id) {
            self.work.files.pending.push(file.id.clone());
            cx.emit(WorkEvent::AttachLibraryFile(file));
        }
        cx.notify();
    }

    pub(super) fn close_work_menus(&mut self) {
        self.work.files.open = false;
        self.work.plugins.open = false;
        self.work.plugins.connect_open = false;
        self.work.run_open = false;
    }

    /// Opens or closes `Files` (`None` toggles); opening clears the search and reads the
    /// recent files, as `refetchOnMount: always` does.
    pub fn set_files_menu(
        &mut self,
        open: Option<bool>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let open = open.unwrap_or(!self.work.files.open);
        self.close_work_menus();
        self.work.files.open = open;
        if open {
            let search = self.work.files.search.clone();
            search.update(cx, |search, cx| search.set_text("", cx));
            self.work.files.typed.clear();
            self.work.files.debounce = None;
            window.focus(&search.focus_handle(cx), cx);
            self.request_library_files(String::new(), cx);
        }
        cx.notify();
    }

    /// Opens or closes `Plugins` (`None` toggles); opening reads `plugin/list` again.
    pub fn set_plugins_menu(
        &mut self,
        open: Option<bool>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let open = open.unwrap_or(!self.work.plugins.open);
        self.close_work_menus();
        self.work.plugins.open = open;
        if open {
            let search = self.work.plugins.search.clone();
            search.update(cx, |search, cx| search.set_text("", cx));
            window.focus(&search.focus_handle(cx), cx);
            match self.composer.read(cx).connected_plugins(cx) {
                Some(list) => {
                    cx.spawn(async move |main, cx| {
                        let list = list.await.map_err(|error| error.to_string());
                        main.update(cx, |main, cx| {
                            main.work.plugins.load = Some(list);
                            cx.notify();
                        })
                        .ok();
                    })
                    .detach();
                }
                None => self.work.plugins.load = Some(Err("no app-server".into())),
            }
        }
        cx.notify();
    }

    /// A plugin row (`onPluginSelectionChange`): picks add a chip, unchecks remove its chips;
    /// the menu closes and the composer takes focus.
    fn pick_plugin(
        &mut self,
        plugin: ConnectedPlugin,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let selected = self
            .composer
            .read(cx)
            .mentioned_plugins()
            .contains(&plugin.id);
        self.composer.update(cx, |c, cx| {
            if selected {
                c.remove_plugin_mentions(&plugin.id, cx);
            } else {
                c.insert_plugin_mention(&plugin, cx);
            }
        });
        self.close_work_menus();
        window.focus(&self.composer.focus_handle(cx), cx);
        cx.notify();
    }

    pub fn set_run_menu(&mut self, open: Option<bool>, cx: &mut Context<Self>) {
        let open = open.unwrap_or(!self.work.run_open);
        self.close_work_menus();
        self.work.run_open = open;
        cx.notify();
    }

    /// The open rail menu in its trigger's slot: `Files`, `Plugins`, run location.
    pub(super) fn work_menus(
        &self,
        t: &Theme,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> [Option<AnyElement>; 3] {
        [
            self.work.files.open.then(|| self.files_menu(t, window, cx)),
            self.work
                .plugins
                .open
                .then(|| self.plugins_menu(t, window, cx)),
            self.work.run_open.then(|| self.run_menu(t, window, cx)),
        ]
    }

    fn files_menu(&self, t: &Theme, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let menu = &self.work.files;
        let typed = menu.search.read(cx).field_text().trim().to_owned();
        // Until the debounced query answers, the shown files narrow locally (`Ve`).
        let settling = matches!(menu.load, LibraryLoad::Loading) || typed != menu.query;
        let files = match &menu.load {
            LibraryLoad::Ready(files) if settling && !typed.is_empty() => {
                Some(local_matches(files, &typed))
            }
            LibraryLoad::Ready(files) => Some(files.clone()),
            _ => None,
        };
        let loading = settling && files.as_ref().is_none_or(Vec::is_empty);
        let list = div().flex().flex_col();
        let list = if loading {
            list.children((0..SKELETON_ROWS).map(|i| skeleton_row(i, t)))
        } else if menu.load == LibraryLoad::Failed {
            list.child(message("Couldn't load Library files", t.text_error))
        } else {
            let files = files.unwrap_or_default();
            if files.is_empty() {
                list.child(message(
                    if typed.is_empty() {
                        "No recent files"
                    } else {
                        "No matching files"
                    },
                    t.text_tertiary,
                ))
            } else {
                list.children(files.into_iter().enumerate().map(|(i, file)| {
                    let checked = self.library_file_attached(&file.id, cx).is_some();
                    let enabled = !menu.pending.contains(&file.id) && file.attachable();
                    let owner = cx.entity();
                    let name = SharedString::from(file.name.clone());
                    menu_row(("library-file", i), "context-file-document", name, t)
                        .role(gpui::Role::MenuItemCheckBox)
                        .aria_toggled(toggled(checked))
                        .aria_disabled(!enabled)
                        .when(!enabled, |d| d.opacity(0.5))
                        .child(check(checked, t))
                        .when(enabled || checked, |d| {
                            d.on_click(move |_, _, cx| {
                                owner.update(cx, |main, cx| {
                                    main.toggle_library_file(file.clone(), cx)
                                })
                            })
                        })
                }))
            }
        };
        let owner = cx.entity();
        let popup = crate::composer::popover(t, 280., |d| {
            d.child(search_row(&menu.search, t))
                .child(separator(t))
                .child(
                    div()
                        .id("library-files")
                        .max_h(px(7. * 28.5))
                        .overflow_y_scroll()
                        .child(list),
                )
        })
        .id("work-files-menu")
        .debug_selector(|| "work-files-menu".into())
        .role(gpui::Role::Menu)
        .aria_label("Files");
        let close = move |window: &mut Window, cx: &mut App| {
            owner.update(cx, |main, cx| main.set_files_menu(Some(false), window, cx))
        };
        dropdown(popup, None, close, window)
    }

    fn plugins_menu(&self, t: &Theme, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let menu = &self.work.plugins;
        let query = menu.search.read(cx).field_text().trim().to_lowercase();
        let mentioned = self.composer.read(cx).mentioned_plugins();
        let matches: Vec<ConnectedPlugin> = match &menu.load {
            Some(Ok(plugins)) => plugins
                .iter()
                .filter(|p| p.title.to_lowercase().contains(&query))
                .cloned()
                .collect(),
            _ => Vec::new(),
        };
        let first = matches.first().cloned();
        let owner = cx.entity();
        let rows: Vec<AnyElement> = matches
            .into_iter()
            .enumerate()
            .map(|(i, plugin)| {
                let checked = mentioned.contains(&plugin.id);
                let owner = owner.clone();
                menu_row(
                    ("work-plugin", i),
                    "plugins",
                    plugin.title.clone().into(),
                    t,
                )
                .role(gpui::Role::MenuItemCheckBox)
                .aria_toggled(toggled(checked))
                .when(i == 0, |d| d.bg(t.menu_item_hover))
                .child(check(checked, t))
                .on_click(move |_, window, cx| {
                    owner.update(cx, |main, cx| main.pick_plugin(plugin.clone(), window, cx))
                })
                .into_any_element()
            })
            .collect();
        let status = match &menu.load {
            None => Some(message("Loading plugins…", t.text_tertiary)),
            Some(Err(_)) => Some(message("Couldn't load plugins", t.text_error)),
            Some(Ok(_)) if rows.is_empty() => Some(message(
                if query.is_empty() {
                    "No connected plugins"
                } else {
                    "No plugins found"
                },
                t.text_tertiary,
            )),
            Some(Ok(_)) => None,
        };
        let submenu = menu.connect_open.then(|| {
            let owner = owner.clone();
            let browse = menu_row(
                ("work-plugins-browse", 0),
                "plugins",
                "Browse all plugins".into(),
                t,
            )
            .role(gpui::Role::MenuItem)
            .on_click(move |_, _, cx| {
                owner.update(cx, |main, cx| {
                    main.close_work_menus();
                    cx.emit(WorkEvent::BrowsePlugins);
                    cx.notify();
                })
            });
            let flyout = crate::composer::popover(t, 240., |d| d.child(browse))
                .id("work-plugins-connect-menu")
                .role(gpui::Role::Menu)
                .aria_label("Connect plugins")
                .occlude()
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation());
            // The flyout's origin is the row's top-right corner, less the shadow inset.
            div().absolute().top_0().right_0().child(
                deferred(
                    anchored()
                        .position_mode(gpui::AnchoredPositionMode::Local)
                        .position(point(px(0.), px(-16.5)))
                        .snap_to_window_with_margin(px(8.))
                        .child(flyout),
                )
                .with_priority(6),
            )
        });
        let connect = {
            let owner = owner.clone();
            menu_row(
                ("work-plugins-connect", 0),
                "settings-mcp-plus",
                "Connect plugins".into(),
                t,
            )
            .relative()
            .role(gpui::Role::MenuItem)
            .aria_expanded(menu.connect_open)
            .when(menu.connect_open, |d| d.bg(t.menu_item_hover))
            .child(icon("chevron-right", 16., t.text_tertiary))
            .children(submenu)
            .on_hover({
                let owner = owner.clone();
                move |hovered, _, cx| {
                    if *hovered {
                        owner.update(cx, |main, cx| {
                            main.work.plugins.connect_open = true;
                            cx.notify();
                        })
                    }
                }
            })
            .on_click(move |_, _, cx| {
                owner.update(cx, |main, cx| {
                    main.work.plugins.connect_open = true;
                    cx.notify();
                })
            })
        };
        let enter_owner = owner.clone();
        let popup = crate::composer::popover(t, 240., |d| {
            d.child(search_row(&menu.search, t))
                .child(
                    div()
                        .id("work-plugins")
                        .max_h(px(250.))
                        .overflow_y_scroll()
                        .children(rows)
                        .children(status),
                )
                .child(separator(t))
                .child(connect)
        })
        .id("work-plugins-menu")
        .role(gpui::Role::Menu)
        .aria_label("Plugins")
        // Enter picks the highlighted (first) plugin, as cmdk does.
        .capture_action(move |_: &crate::composer::Submit, window, cx| {
            cx.stop_propagation();
            if let Some(plugin) = first.clone() {
                enter_owner.update(cx, |main, cx| main.pick_plugin(plugin, window, cx));
            }
        });
        let close = move |window: &mut Window, cx: &mut App| {
            owner.update(cx, |main, cx| {
                main.set_plugins_menu(Some(false), window, cx)
            })
        };
        dropdown(popup, None, close, window)
    }

    /// `Where should this chat run?`: this app runs Work chats on this computer only; cloud
    /// Work needs the ChatGPT cloud runner, so `In the cloud` stays disabled.
    fn run_menu(&self, t: &Theme, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let owner = cx.entity();
        let option = |id: &'static str, glyph, label: &'static str, sub: &'static str, enabled| {
            div()
                .id(id)
                .debug_selector(move || id.into())
                .role(gpui::Role::MenuItemRadio)
                .aria_label(label)
                .aria_toggled(toggled(enabled))
                .aria_disabled(!enabled)
                .px(px(8.))
                .py(px(6.))
                .flex()
                .items_start()
                .gap(px(8.))
                .rounded(px(12.))
                .when(!enabled, |d| d.opacity(0.5))
                .when(enabled, |d| {
                    let hover = t.menu_item_hover;
                    d.hover(move |s| s.bg(hover))
                })
                .child(div().py(px(1.)).child(icon(glyph, 16., t.text)))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .child(text(label, ROW_TEXT.0, ROW_TEXT.1, ROW_TEXT.2, t.text))
                        .child(text(sub, 12., 16., 400., t.text_tertiary)),
                )
                .child(div().py(px(1.)).child(check(enabled, t)))
        };
        let local = {
            let owner = owner.clone();
            option(
                "run-location-local",
                "laptop",
                "On your computer",
                "This chat stays on this computer",
                true,
            )
            .on_click(move |_, _, cx| {
                owner.update(cx, |main, cx| main.set_run_menu(Some(false), cx))
            })
        };
        let cloud = option(
            "run-location-cloud",
            "settings-codex-cloud",
            "In the cloud",
            "Can't access local files unless attached",
            false,
        );
        let popup = crate::composer::popover(t, 300., |d| {
            d.child(div().px(px(8.)).py(px(4.)).child(text(
                "Where should this chat run?",
                ROW_TEXT.0,
                ROW_TEXT.1,
                ROW_TEXT.2,
                t.text_tertiary,
            )))
            .child(local)
            .child(cloud)
        })
        .id("run-location-menu")
        .role(gpui::Role::Menu)
        .aria_label("Where should this chat run?");
        let close = move |_: &mut Window, cx: &mut App| {
            owner.update(cx, |main, cx| main.set_run_menu(Some(false), cx))
        };
        dropdown(popup, Some(RUN_BUTTON), close, window)
    }
}

/// `Ve`: the files whose names (without spaces, `_` and `-`) hold every word of `query`;
/// with none, the query less its last character.
pub fn local_matches(files: &[LibraryFile], query: &str) -> Vec<LibraryFile> {
    let find = |query: &str| -> Vec<LibraryFile> {
        let words: Vec<String> = query
            .to_lowercase()
            .split(|c: char| c.is_whitespace() || c == '_' || c == '-')
            .filter(|w| !w.is_empty())
            .map(str::to_owned)
            .collect();
        if words.is_empty() {
            return Vec::new();
        }
        files
            .iter()
            .filter(|file| {
                let name: String = file
                    .name
                    .to_lowercase()
                    .chars()
                    .filter(|c| !(c.is_whitespace() || *c == '_' || *c == '-'))
                    .collect();
                words.iter().all(|w| name.contains(w.as_str()))
            })
            .cloned()
            .collect()
    };
    let found = find(query);
    if !found.is_empty() {
        return found;
    }
    let mut shorter = query.chars();
    shorter.next_back();
    find(shorter.as_str())
}

/// The rail's `Files`, `Plugins` and run-location triggers, each holding its open menu
/// (`menus`, from [`HomeMain::work_menus`]) so the menu lays out with it in the same frame.
pub(super) fn rail_controls(
    owner: &Entity<HomeMain>,
    menus: [Option<AnyElement>; 3],
    compact: bool,
    t: &Theme,
    window: &mut Window,
    cx: &mut App,
) -> (Vec<AnyElement>, AnyElement) {
    let [files_menu, plugins_menu, run_menu] = menus;
    // The menu's origin is its trigger's top-left corner.
    let slot =
        |menu: Option<AnyElement>| menu.map(|menu| div().absolute().top_0().left_0().child(menu));
    let pill = |id: &'static str,
                glyph: &'static str,
                label: &'static str,
                tip: &'static str,
                menu: Option<AnyElement>,
                toggle: fn(&mut HomeMain, &mut Window, &mut Context<HomeMain>),
                window: &mut Window,
                cx: &mut App| {
        let focus = focus::handle(id, 0, window, cx);
        let toggle_owner = owner.clone();
        let key_owner = owner.clone();
        let handle = focus.read(cx).handle.clone();
        let open = menu.is_some();
        let chip = chip(id, glyph, label.into(), t.text, compact, t, &focus, cx)
            .role(gpui::Role::Button)
            .debug_selector(move || id.into())
            .aria_label(label)
            .aria_expanded(open)
            .when(open, |d| d.bg(t.ghost_hover).rounded_full())
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(move |_, window, cx| {
                toggle_owner.update(cx, |main, cx| toggle(main, window, cx))
            })
            .on_key_down(move |event, window, cx| {
                // The open menu's search field sits inside the trigger: its keys bubble here.
                if handle.is_focused(window)
                    && matches!(event.keystroke.key.as_str(), "enter" | "space")
                {
                    key_owner.update(cx, |main, cx| toggle(main, window, cx));
                    cx.stop_propagation();
                }
            })
            .children(slot(menu));
        crate::tooltip::tooltip(chip, tip, None, crate::tooltip::Side::Top)
            .focus_handle(&focus.read(cx).handle)
            .into_any_element()
    };
    let files = pill(
        "work-home-files-control",
        "books",
        "Files",
        "Files",
        files_menu,
        |main, window, cx| main.set_files_menu(None, window, cx),
        window,
        cx,
    );
    let plugins = pill(
        "composer-work-home-plugins-control",
        "sidebar-cmd-plugins",
        "Plugins",
        "Work with ChatGPT across your favorite tools",
        plugins_menu,
        |main, window, cx| main.set_plugins_menu(None, window, cx),
        window,
        cx,
    );
    let run = {
        let id = "composer-action-bar-run-location-dropdown";
        let focus = focus::handle(id, 0, window, cx);
        let toggle_owner = owner.clone();
        let key_owner = owner.clone();
        let handle = focus.read(cx).handle.clone();
        let open = run_menu.is_some();
        let button = focus::shadow_ring(
            div().id((id, 0_usize)).relative().flex_none(),
            &focus,
            Theme::RADIUS_FULL,
            px(2.),
            cx,
        )
        .role(gpui::Role::Button)
        .debug_selector(move || id.into())
        .aria_label("Choose where to run this chat")
        .aria_expanded(open)
        .size(RUN_BUTTON)
        .flex()
        .items_center()
        .justify_center()
        .rounded_full()
        .when(open, |d| d.bg(t.ghost_hover))
        .hover(|s| s.bg(t.ghost_hover))
        .child(icon("laptop", 16., t.text))
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_click(move |_, _, cx| toggle_owner.update(cx, |main, cx| main.set_run_menu(None, cx)))
        .on_key_down(move |event, window, cx| {
            if handle.is_focused(window)
                && matches!(event.keystroke.key.as_str(), "enter" | "space")
            {
                key_owner.update(cx, |main, cx| main.set_run_menu(None, cx));
                cx.stop_propagation();
            }
        })
        .children(slot(run_menu));
        crate::tooltip::tooltip(
            button,
            "Choose where to run this chat",
            None,
            crate::tooltip::Side::Top,
        )
        .focus_handle(&focus.read(cx).handle)
        .into_any_element()
    };
    (vec![files, plugins], run)
}

fn menu_row(
    id: (&'static str, usize),
    glyph: &'static str,
    label: SharedString,
    t: &Theme,
) -> Stateful<Div> {
    let hover = t.menu_item_hover;
    div()
        .id(id)
        .debug_selector(move || format!("{}-{}", id.0, id.1))
        .aria_label(label.clone())
        .px(px(8.))
        .py(px(5.))
        .flex()
        .items_center()
        .gap(px(6.))
        .rounded(px(15.))
        .hover(move |s| s.bg(hover))
        .child(icon(glyph, 16., t.text).opacity(0.75))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .overflow_hidden()
                .child(text(label, ROW_TEXT.0, ROW_TEXT.1, ROW_TEXT.2, t.text)),
        )
}

fn toggled(on: bool) -> gpui::Toggled {
    if on {
        gpui::Toggled::True
    } else {
        gpui::Toggled::False
    }
}

fn check(checked: bool, t: &Theme) -> Div {
    div()
        .size(px(16.))
        .children(checked.then(|| icon("menu-check", 16., t.text)))
}

fn message(label: &'static str, color: gpui::Hsla) -> Div {
    div()
        .px(px(8.))
        .py(px(5.))
        .child(text(label, ROW_TEXT.0, ROW_TEXT.1, ROW_TEXT.2, color))
}

/// A disabled `Skeleton` row, 60 % and 80 % wide in turn.
fn skeleton_row(index: usize, t: &Theme) -> Div {
    div().px(px(8.)).py(px(10.)).child(
        div()
            .h(px(8.))
            .w(gpui::relative(if index.is_multiple_of(2) {
                0.6
            } else {
                0.8
            }))
            .rounded_full()
            .bg(t.ghost_hover),
    )
}

fn search_row(search: &Entity<Composer>, t: &Theme) -> Div {
    div()
        .px(px(8.))
        .py(px(5.))
        .flex()
        .items_center()
        .gap(px(6.))
        .child(icon("shortcut-search", 14., t.text_tertiary))
        .child(div().flex_1().min_w_0().child(search.clone()))
}

fn separator(t: &Theme) -> Div {
    div()
        .px(px(8.))
        .py(px(4.))
        .child(div().h(px(1.)).bg(t.border))
}

/// `popup` opening upward from its trigger (`side: top`), start-aligned or, with the trigger's
/// width `end`, end-aligned; placed in the trigger's slot, it lays out with the trigger. A
/// full-window backdrop under it closes it on a click elsewhere; Escape closes it too.
fn dropdown(
    popup: Stateful<Div>,
    end: Option<Pixels>,
    close: impl Fn(&mut Window, &mut App) + Clone + 'static,
    window: &Window,
) -> AnyElement {
    let keys = close.clone();
    let popup = popup
        .occlude()
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .capture_key_down(move |event, window, cx| {
            if event.keystroke.key == "escape" {
                cx.stop_propagation();
                keys(window, cx);
            }
        });
    // `sideOffset` 0 plus the content's `m-px`, less the popover's shadow-layer inset: the
    // `Choose project` menu's measured anchor (its bounds canvas sits one 28 px pill lower).
    let (x, anchor) = match end {
        Some(width) => (width - px(1.25), gpui::Anchor::BottomRight),
        None => (px(1.25), gpui::Anchor::BottomLeft),
    };
    let backdrop = deferred(
        anchored().position(point(px(0.), px(0.))).child(
            div()
                .w(window.viewport_size().width)
                .h(window.viewport_size().height)
                .occlude()
                .on_mouse_down(MouseButton::Left, move |_, window, cx| close(window, cx)),
        ),
    )
    .with_priority(4);
    let menu = deferred(
        anchored()
            .position_mode(gpui::AnchoredPositionMode::Local)
            .position(point(x, px(28. - 18.1)))
            .anchor(anchor)
            .snap_to_window_with_margin(px(8.))
            .child(popup),
    )
    .with_priority(5);
    div().child(backdrop).child(menu).into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(name: &str, size: Option<u64>) -> LibraryFile {
        LibraryFile {
            id: format!("libfile_{name}"),
            file_id: format!("file_{name}"),
            name: name.into(),
            size,
        }
    }

    #[test]
    fn local_matches_ignore_separators_and_drop_a_trailing_typo() {
        let files = [
            file("Q3 plan_final.pdf", Some(1)),
            file("notes.txt", Some(1)),
        ];
        let names = |found: Vec<LibraryFile>| found.into_iter().map(|f| f.name).collect::<Vec<_>>();
        assert_eq!(
            names(local_matches(&files, "planfinal")),
            ["Q3 plan_final.pdf"]
        );
        assert_eq!(
            names(local_matches(&files, "q3 final")),
            ["Q3 plan_final.pdf"]
        );
        // `notesx` finds nothing, so `notes` is tried.
        assert_eq!(names(local_matches(&files, "notesx")), ["notes.txt"]);
        assert!(local_matches(&files, "zz").is_empty());
    }

    #[test]
    fn only_known_sizes_up_to_the_local_limit_attach() {
        assert!(file("a", Some(LOCAL_FILE_LIMIT)).attachable());
        assert!(!file("a", Some(LOCAL_FILE_LIMIT + 1)).attachable());
        assert!(!file("a", None).attachable());
    }
}
