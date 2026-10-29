use crate::*;
use gpui::Div;

mod archive;
mod automations;
mod home;
mod mobile;
mod plugins;
mod settings;
mod thread;

pub(crate) use settings::request_quit;
pub(crate) use thread::{forked_thread, load_thread};

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub enum Route {
    #[default]
    Home,
    Thread(String),
    Remote {
        task_id: String,
        hotkey: bool,
    },
    Settings,
    SettingsSubtree(String),
    CloudEnvironments(String),
    CloudSetup {
        config_id: String,
        repositories: Vec<Value>,
    },
    /// Rail `Scheduled` (`/scheduled`): the Automations page.
    Scheduled,
    /// A sidebar task row (`/scheduled?automationId=<id>`): the task's thread.
    Automation(String),
    /// Profile menu ▸ `Set up mobile` (`/codex-mobile`).
    Mobile,
    Security,
    Plugins(String),
    Archive,
    PullRequests(Option<ui::screens::pull_requests::model::Identity>),
    SelectWorkspace,
    Onboarding(ui::screens::onboarding::Page),
}

pub(crate) fn actions(mut root: Div, cx: &mut Context<Root>) -> Div {
    root = home::actions(root, cx);
    root = thread::actions(root, cx);
    root = settings::actions(root, cx);
    root = automations::actions(root, cx);
    root = mobile::actions(root, cx);
    root = crate::security::actions(root, cx);
    root = plugins::actions(root, cx);
    root = archive::actions(root, cx);
    root = crate::pull_requests::actions(root, cx);
    root
}

impl Root {
    pub(crate) fn navigate(&mut self, route: Route, window: &mut Window, cx: &mut Context<Self>) {
        let refresh = route == Route::Scheduled && self.set_automation_source("local");
        let same_route = self.state.route == route;
        self.navigate_with_sidebar(route, None, window, cx);
        if refresh && same_route {
            self.sync(window, cx);
        }
    }

    pub(crate) fn navigate_with_sidebar(
        &mut self,
        route: Route,
        automation: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if route != self.state.route || automation != self.sidebar_automation {
            if self.instructions_changed(cx) {
                self.confirm_instructions_leave(
                    SettingsLeave::Route(route, automation),
                    window,
                    cx,
                );
                return;
            }
            ui::composer::dictation::Dispatcher::cancel(window.window_handle().window_id(), cx);
            self.remember_thread_settings(cx);
            self.back.push((
                std::mem::replace(&mut self.state.route, route),
                std::mem::replace(&mut self.sidebar_automation, automation),
            ));
            self.navigation_epoch += 1;
            self.forward.clear();
            self.sync(window, cx);
        }
        self.select_route(cx);
    }

    pub(crate) fn step(&mut self, back: bool, window: &mut Window, cx: &mut Context<Self>) {
        if back && let Some(page) = &self.settings_page {
            let editor = page.read(cx).personalization.clone();
            if editor.read(cx).opened() {
                if self.instructions_changed(cx) {
                    self.confirm_instructions_leave(SettingsLeave::Editor, window, cx);
                } else {
                    editor.update(cx, |editor, cx| editor.discard(cx));
                }
                return;
            }
        }
        if self.instructions_changed(cx) {
            self.confirm_instructions_leave(SettingsLeave::History(back), window, cx);
            return;
        }
        self.remember_thread_settings(cx);
        let (from, to) = if back {
            (&mut self.back, &mut self.forward)
        } else {
            (&mut self.forward, &mut self.back)
        };
        if let Some((route, automation)) = from.pop() {
            ui::composer::dictation::Dispatcher::cancel(window.window_handle().window_id(), cx);
            to.push((
                std::mem::replace(&mut self.state.route, route),
                std::mem::replace(&mut self.sidebar_automation, automation),
            ));
            self.navigation_epoch += 1;
            self.select_route(cx);
            self.sync(window, cx);
        }
    }

    pub(crate) fn select_route(&mut self, cx: &mut Context<Self>) {
        let id = match &self.state.route {
            Route::Thread(id) => Some(id.clone()),
            _ => None,
        };
        self.sidebar.update(cx, |s, cx| s.select(id.as_deref(), cx));
        if let Some(id) = id {
            self.read_automation_run(&id, cx);
        }
    }

    /// Shows the route (loading its thread if needed), pushes history into the shell and
    /// persists the state.
    pub(crate) fn sync(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        cx.set_global(ui::screens::settings::AppServer(self.bridge.clone()));
        crate::connections::sync(cx);
        let home = matches!(self.state.route, Route::Home);
        if cx.has_global::<ui::screens::settings::keyboard_shortcuts::KeyboardState>() {
            cx.update_global::<ui::screens::settings::keyboard_shortcuts::KeyboardState, _>(
                |state, _| state.home = home,
            );
        }
        self.sync_native_menus(window, cx);
        self.remember_recent_chat();
        self.flush_settings(cx);
        self.sync_diff_watch(cx);
        ui::screens::settings::imports::deactivate(cx);
        if self.ready
            && matches!(
                self.state.route,
                Route::Settings | Route::SettingsSubtree(_)
            )
            && cx
                .try_global::<ui::screens::settings::imports::ImportState>()
                .is_some_and(|state| state.policy_request == 0)
        {
            self.load_import_policy(cx);
        }
        self.settings_page = None;
        self.suggestion_load = None;
        self.oauth.leave_page();
        let (can_back, can_forward) = (!self.back.is_empty(), !self.forward.is_empty());
        self.archive_page = None;
        self.archived_settings = None;
        self.mobile_page = None;
        self.cloud = None;
        self.plugins_page = None;
        self.onboarding_page = None;
        self.cloud_modal = None;
        self.page = match self.state.route.clone() {
            Route::Remote { task_id, hotkey } => {
                self.load_cloud_task(task_id, hotkey, window, cx);
                None
            }
            Route::CloudSetup {
                config_id,
                repositories,
            } => {
                self.load_cloud_environment(config_id, repositories, window, cx);
                None
            }
            Route::Settings | Route::SettingsSubtree(_) | Route::CloudEnvironments(_) => {
                Some(settings::page(self, window, cx))
            }
            Route::Scheduled => Some(automations::scheduled(self, cx)),
            Route::Automation(id) => Some(automations::automation(self, id, window, cx)),
            Route::Archive => Some(archive::page(self, cx)),
            Route::Thread(_) if self.sidebar_automation.is_some() => {
                Some(automations::thread(self, cx))
            }
            Route::Mobile => Some(mobile::page(self, cx)),
            Route::PullRequests(selected) => Some(self.pull_requests_shell(selected, window, cx)),
            Route::Security => Some(crate::security::shell(self, window, cx)),
            Route::Plugins(path) => Some(self.plugins_shell(path, window, cx)),
            Route::Onboarding(page) => {
                self.open_onboarding(page, window, cx);
                None
            }
            Route::Home | Route::Thread(_) | Route::SelectWorkspace => None,
        };
        if let Some(sidebar) = &self.scheduled {
            for id in self.automation_updates.keys() {
                sidebar.update(cx, |sidebar, cx| sidebar.set_pending(id, true, cx));
            }
        }
        if self.scheduled_route() {
            self.load_suggestions(cx);
        }
        if let Some(page) = &self.page {
            page.update(cx, |shell, cx| {
                shell.can_go_back = can_back;
                shell.can_go_forward = can_forward;
                shell.unread_destinations = self.unread_destinations.clone();
                let access = self.security_access.read(cx);
                shell.security_available = access.gate_enabled && access.plugin_available;
            });
        }
        self.sync_automation_attention(cx);
        let thread_id = match &self.state.route {
            Route::Thread(id) => Some(id.clone()),
            _ => None,
        };
        if self
            .history
            .as_ref()
            .is_some_and(|(id, _)| Some(id) != thread_id.as_ref())
        {
            self.history = None;
        }
        let view = self
            .thread
            .clone()
            .filter(|v| Some(v.read(cx).id()) == thread_id.as_deref());
        let first_run = self
            .onboarding_page
            .clone()
            .filter(|page| page.read(cx).page() == ui::screens::onboarding::Page::FirstRun);
        let main: AnyView = match (&thread_id, &view) {
            _ if first_run.is_some() => first_run.clone().unwrap().into(),
            _ if self.cloud.is_some() => self.cloud.as_ref().unwrap().page.clone().into(),
            (Some(_), Some(view)) => view.clone().into(),
            (Some(id), None) => self.load_thread(id.clone(), window, cx).into(),
            (None, _) => self.home.clone().into(),
        };
        let shell = if self.sidebar_automation.is_some() {
            self.page.clone().unwrap_or_else(|| self.shell.clone())
        } else {
            self.shell.clone()
        };
        let home = matches!(
            self.state.route,
            Route::Home | Route::Thread(_) | Route::Remote { .. } | Route::CloudSetup { .. }
        );
        shell.update(cx, |shell, cx| {
            shell.selected_rail = if self.sidebar_automation.is_some() {
                Some("scheduled")
            } else {
                home.then_some("home")
            };
            shell.can_go_back = can_back;
            shell.can_go_forward = can_forward;
            // Thread routes: `with_side_panel` below sets its own.
            shell.new_tab = first_run.is_none();
            shell.chat_chrome = first_run.is_none();
            shell.main_composited = false;
            shell.set_main(main, cx);
        });
        self.split = view.map(|view| {
            let panel = self.side_tabs_view(&view, cx);
            with_side_tabs(shell.clone(), view, panel, self.workspace_layout(), cx)
        });
        window.focus(&self.route_focus(cx), cx);
        if let Some(id) = thread_id
            && self
                .thread
                .as_ref()
                .is_some_and(|view| view.read(cx).id() == id)
        {
            self.list_queue(id, cx);
        }
        self.sync_terminal(cx);
        self.save(window, cx);
        cx.notify();
    }

    /// Keys need a focused element in the frame: the route's composer, else the root (the
    /// route's own shell, a loading thread, the sign-in view, which take no keys of their own).
    pub(crate) fn route_focus(&self, cx: &App) -> FocusHandle {
        let open = |id: &str| self.thread.as_ref().is_some_and(|v| v.read(cx).id() == id);
        match &self.state.route {
            _ if self.login.is_some() => self.focus.clone(),
            Route::Remote { .. } | Route::CloudSetup { .. } => self
                .cloud
                .as_ref()
                .map(|session| {
                    let page = session.page.read(cx);
                    if page.has_composer() {
                        page.composer.focus_handle(cx)
                    } else {
                        page.focus_handle(cx)
                    }
                })
                .unwrap_or_else(|| self.focus.clone()),
            Route::Thread(id) if open(id) => {
                if self.workspace_layout() == WorkspaceLayout::Full
                    && let Some(split) = &self.split
                {
                    split.focus_handle(cx)
                } else {
                    self.thread_composer.focus_handle(cx)
                }
            }
            Route::Onboarding(_) => self.focus.clone(),
            _ if self.page.is_some() => self.focus.clone(),
            Route::Thread(_) => self.focus.clone(),
            _ => self.home.read(cx).composer.focus_handle(cx),
        }
    }
}
