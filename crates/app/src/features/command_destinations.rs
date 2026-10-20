use crate::*;
use gpui::BorrowAppContext as _;

actions!(app, [SearchFiles, OpenThreadInNewWindow]);

const FILE_HISTORY: &str = "recent-command-files";

pub(super) fn init(cx: &mut App) {
    cx.bind_keys([KeyBinding::new("cmd-p", SearchFiles, None)]);
}

pub(super) fn actions(root: gpui::Div, cx: &mut Context<Root>) -> gpui::Div {
    root.on_action(cx.listener(|this, _: &SearchFiles, window, cx| {
        this.open_file_search(window, cx);
    }))
    .on_action(cx.listener(|this, _: &OpenThreadInNewWindow, _, cx| {
        if let Route::Thread(id) = &this.state.route {
            this.open_thread_window(id.clone(), cx);
        }
    }))
}

fn recent_files(root: &str, cx: &App) -> Vec<String> {
    cx.try_global::<host_settings::Settings>()
        .and_then(|settings| settings.read_all().get(FILE_HISTORY))
        .and_then(|history| history.get(root))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect()
}

impl Root {
    pub(crate) fn open_file_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(thread) = &self.thread else { return };
        let cwd = std::path::Path::new(thread.read(cx).cwd())
            .components()
            .as_path()
            .to_string_lossy()
            .into_owned();
        let query = thread
            .read(cx)
            .selected_text(window, cx)
            .unwrap_or_default();
        let recent = recent_files(&cwd, cx);
        let menu = cx.new(|cx| CommandMenu::files(&cwd, recent, query.trim(), cx));
        let bridge = self.bridge.clone();
        // The App owns cleanup because its Root can be released first.
        App::observe_release(cx, &menu, move |menu, cx| {
            if !menu.file_session_started() {
                return;
            }
            let session = menu.file_session().unwrap().to_owned();
            cx.foreground_executor()
                .spawn(async move {
                    if let Err(error) = bridge
                        .request::<_, Value>(
                            "fuzzyFileSearch/sessionStop",
                            json!({"sessionId": session}),
                        )
                        .await
                    {
                        eprintln!("codex-gpui: cannot stop file search: {error:#}");
                    }
                })
                .detach();
        })
        .detach();
        cx.subscribe_in(&menu, window, |this, _, event, window, cx| {
            this.run_command(event, window, cx);
        })
        .detach();
        window.focus(&menu.focus_handle(cx), cx);
        self.command_menu = Some(menu);
        if !query.trim().is_empty() {
            self.search_command_files(query.trim().to_owned(), cx);
        }
        cx.notify();
    }

    pub(crate) fn search_command_files(&self, query: String, cx: &mut Context<Self>) {
        let Some(menu) = self.command_menu.clone() else {
            return;
        };
        let Some(session) = menu.read(cx).file_session().map(str::to_owned) else {
            return;
        };
        let absolute = query.starts_with('/') || query.starts_with("~/");
        let start = if absolute {
            false
        } else {
            let Some(start) = menu.update(cx, |menu, _| menu.prepare_file_search()) else {
                return;
            };
            start
        };
        menu.update(cx, |menu, cx| menu.start_file_lookup(cx));
        let roots = if start {
            vec![menu.read(cx).file_root().unwrap().to_owned()]
        } else {
            Vec::new()
        };
        let menu = menu.downgrade();
        let bridge = self.bridge.clone();
        let task = cx.spawn(async move |this, cx| -> Result<()> {
            if menu.upgrade().is_none() {
                return Ok(());
            }
            if start {
                let result = bridge
                    .request::<_, Value>(
                        "fuzzyFileSearch/sessionStart",
                        json!({"sessionId": session, "roots": roots}),
                    )
                    .await;
                match menu.update(cx, |menu, cx| {
                    menu.finish_file_start(&session, result.is_ok(), cx)
                }) {
                    Ok(Some(query)) if result.is_ok() && !query.is_empty() => {
                        this.update(cx, |this, cx| {
                            if this
                                .command_menu
                                .as_ref()
                                .and_then(|menu| menu.read(cx).file_session())
                                == Some(session.as_str())
                            {
                                this.search_command_files(query, cx);
                            }
                        })
                        .ok();
                    }
                    Err(_) if result.is_ok() => {
                        bridge
                            .request::<_, Value>(
                                "fuzzyFileSearch/sessionStop",
                                json!({"sessionId": session}),
                            )
                            .await?;
                    }
                    _ => {}
                }
                return result.map(drop);
            }
            if absolute {
                let path = if let Some(relative) = query.strip_prefix("~/") {
                    PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(relative)
                } else {
                    PathBuf::from(&query)
                };
                let result = cx
                    .background_spawn(async move {
                        match fs::metadata(&path) {
                            Ok(metadata) => Ok(metadata
                                .is_file()
                                .then(|| path.to_string_lossy().into_owned())),
                            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
                            Err(error) => Err(error),
                        }
                    })
                    .await;
                menu.update(cx, |menu, cx| {
                    menu.set_file_results(
                        &session,
                        Some(&query),
                        result
                            .as_ref()
                            .ok()
                            .and_then(|path| path.clone())
                            .into_iter()
                            .collect(),
                        result.is_err(),
                        cx,
                    )
                })
                .ok();
                result?;
            } else if let Err(error) = bridge
                .request::<_, Value>(
                    "fuzzyFileSearch/sessionUpdate",
                    json!({"sessionId": session, "query": query}),
                )
                .await
            {
                menu.update(cx, |menu, cx| {
                    menu.set_file_results(&session, Some(&query), Vec::new(), true, cx)
                })
                .ok();
                return Err(error);
            }
            Ok(())
        });
        log_err(task, cx);
    }

    pub(crate) fn remember_file(&self, root: &str, path: &str, cx: &mut Context<Self>) {
        let Some(settings) = cx.try_global::<host_settings::Settings>() else {
            return;
        };
        let mut history = settings
            .read_all()
            .get(FILE_HISTORY)
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        let mut paths = recent_files(root, cx);
        paths.retain(|previous| previous != path);
        paths.insert(0, path.to_owned());
        paths.truncate(10);
        history.insert(root.to_owned(), json!(paths));
        let result = cx.update_global::<host_settings::Settings, _>(|settings, _| {
            settings.write(FILE_HISTORY, Value::Object(history))
        });
        if let Err(error) = result {
            eprintln!("codex-gpui: cannot save recent files: {error:#}");
        }
    }

    fn open_thread_window(&mut self, thread_id: String, cx: &mut Context<Self>) {
        let mut state = self.state.clone();
        state.route = Route::Thread(thread_id);
        state.path = None;
        let bridge = self.bridge.clone();
        let events = bridge.subscribe_async();
        let cwd = self.cwd.clone();
        let host = self.host.clone();
        let approvals = self.approvals.clone();
        let clock = self.clock;
        let bounds =
            gpui::Bounds::centered(None, gpui::size(px(state.width), px(state.height)), cx);
        let mut spec = crate::windows::Spec::new(crate::windows::Kind::Primary, "Codex");
        spec.bounds = Some(gpui::WindowBounds::Windowed(bounds));
        let result = crate::windows::open(spec, cx, |window, cx| {
            cx.new(|cx| {
                let mut root = Root::with_connection(state, bridge, events, cwd, window, cx);
                root.set_host(host, cx);
                root.approvals = approvals;
                root.clock = clock;
                root
            })
        });
        if let Err(error) = result {
            self.toast(
                Toast::danger("Unable to open new window").description(error.to_string()),
                cx,
            );
        }
    }
}
