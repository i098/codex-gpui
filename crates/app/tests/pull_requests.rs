mod common;

use anyhow::{Context as _, Result};
use app::{
    Route,
    host::{Host, ReplayHost, Request, json_body},
};
use common::Fx;
use gpui::Focusable as _;
use std::collections::HashSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use ui::screens::pull_requests::{Availability, DetailTab, Event};

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct RecordedNavigation {
    account_id: String,
    user_id: String,
    device_id: String,
    atoms: serde_json::Value,
}

struct AccountHost {
    replay: ReplayHost,
    switched: AtomicBool,
    review_metadata: serde_json::Value,
    fail_code: AtomicBool,
    navigation: RecordedNavigation,
}

impl Host for AccountHost {
    fn fetch(&self, request: &Request) -> Result<serde_json::Value> {
        if let Request::Fetch { url, body, .. } = request
            && url == "vscode://codex/get-global-state"
            && serde_json::from_str::<serde_json::Value>(body)?["key"]
                == "electron-persisted-atom-state"
        {
            return Ok(serde_json::json!({
                "type": "fetch-response", "responseType": "success", "status": 200, "headers": {},
                "bodyJsonString": serde_json::json!({"value": self.navigation.atoms}).to_string()
            }));
        }
        if let Request::Fetch { url, .. } = request
            && url == "vscode://codex/gh-pr-patch"
        {
            if self.fail_code.load(Ordering::SeqCst) {
                anyhow::bail!("synthetic patch transport failure");
            }
            let diff = "diff --git a/src/example.rs b/src/example.rs\n--- a/src/example.rs\n+++ b/src/example.rs\n@@ -1 +1 @@\n-let ready = false;\n+let ready = true;\n";
            return Ok(serde_json::json!({
                "type":"fetch-response", "responseType":"success", "status":200, "headers":{},
                "bodyJsonString":serde_json::json!({"diff":diff}).to_string()
            }));
        }
        let mut response = self.replay.fetch(request)?;
        if let Request::Fetch { url, .. } = request
            && url == "vscode://codex/gh-pr-detail"
        {
            let mut body = json_body(&response)?;
            let number = body["number"]
                .as_u64()
                .context("recorded detail number")?
                .to_string();
            if let Some(metadata) = self.review_metadata[&number].as_object() {
                body.as_object_mut()
                    .context("recorded detail object")?
                    .extend(metadata.clone());
            }
            response["bodyJsonString"] = body.to_string().into();
        }
        if self.switched.load(Ordering::SeqCst)
            && let Request::Fetch { url, .. } = request
            && url == "vscode://codex/gh-current-user"
        {
            let mut body = json_body(&response)?;
            body["account"]["login"] = "changed-account".into();
            body["login"] = "changed-account".into();
            response["bodyJsonString"] = body.to_string().into();
        }
        Ok(response)
    }

    fn identity(&self) -> Option<app::host::Identity> {
        Some(app::host::Identity {
            account_id: self.navigation.account_id.clone(),
            user_id: self.navigation.user_id.clone(),
            device_id: self.navigation.device_id.clone(),
        })
    }
}

#[test]
fn missing_host_response_keeps_retry_and_chat_available() -> Result<()> {
    let mut fx = Fx::new()?;
    let host = Arc::new(ReplayHost::from_recording(r#"{"exchanges":[]}"#)?);
    fx.root
        .update(&mut fx.cx, |root, cx| root.set_host(host, cx));
    fx.cx.update_window(fx.window, |_, window, cx| {
        window.dispatch_action(Box::new(ui::OpenPullRequests), cx);
    })?;
    fx.wait("GitHub access failure", |fx| {
        fx.read(|root, cx| {
            root.pull_requests_page().is_some_and(|page| {
                matches!(page.read(cx).availability, Availability::Unavailable(_))
            })
        })
    })?;
    assert_eq!(fx.route(), Route::PullRequests(None));
    capture(&mut fx, "access-error")?;
    let page = fx.read(|root, _| root.pull_requests_page().unwrap().clone());
    let generation = fx.read(|_, cx| page.read(cx).generation);
    page.update(&mut fx.cx, |_, cx| cx.emit(Event::Retry));
    fx.wait("GitHub retry failure", |fx| {
        fx.read(|_, cx| {
            page.read(cx).generation > generation
                && matches!(page.read(cx).availability, Availability::Unavailable(_))
        })
    })?;
    assert_eq!(fx.route(), Route::PullRequests(None));
    page.update(&mut fx.cx, |_, cx| {
        cx.emit(Event::AskCodex(
            ui::screens::pull_requests::INSTALL_PROMPT.into(),
        ))
    });
    fx.wait("installation draft", |fx| fx.route() == Route::Home)?;
    assert_eq!(
        fx.composer_text()?,
        ui::screens::pull_requests::INSTALL_PROMPT
    );
    capture(&mut fx, "install-draft")?;
    Ok(())
}

#[test]
fn original_cursor_page_preserves_the_existing_rows() -> Result<()> {
    replay_pane_states(None)
}

#[test]
fn original_pane_reset_keeps_the_mount_default() -> Result<()> {
    for state in [
        "open-all",
        "merged-first-page",
        "merged-second-page",
        "merged-detail",
    ] {
        replay_pane_states(Some(state))?;
    }
    Ok(())
}

fn replay_pane_states(reset_state: Option<&str>) -> Result<()> {
    use ui::screens::pull_requests::model::View;
    let recording =
        std::fs::read_to_string(format!("{}/fixtures/pull-requests.json", common::ROOT))?;
    let mut data: serde_json::Value = serde_json::from_str(&recording)?;
    let mut script = common::script(None, |_| {})?;
    let account = script
        .iter()
        .find(|line| line["msg"]["result"]["account"].is_object())
        .context("recorded account")?["msg"]["result"]
        .clone();
    let catalog = data["navigation"]["catalog"].take();
    // Host and account changes each restart the navigation read, so answer every restart.
    for _ in 0..4 {
        script.extend(common::exchange("account/read", account.clone()));
        let [mut request, response] = common::exchange("mcpServerStatus/list", catalog.clone());
        request["msg"]["params"] =
            serde_json::json!({"cursor": null, "detail": "toolsAndAuthOnly", "limit": 100});
        script.extend([request, response]);
    }
    let mut fx = Fx::scripted(script)?;
    let host = Arc::new(AccountHost {
        replay: ReplayHost::from_recording(&recording)?,
        switched: AtomicBool::new(false),
        review_metadata: data["reviewMetadata"].take(),
        fail_code: AtomicBool::new(true),
        navigation: serde_json::from_value(data["navigation"].take())?,
    });
    fx.root
        .update(&mut fx.cx, |root, cx| root.set_host(host.clone(), cx));
    fx.wait("recorded navigation state", |fx| {
        fx.read(|root, cx| {
            root.shell()
                .read(cx)
                .unread_destinations
                .as_ref()
                .is_some_and(|ids| ids.is_empty())
        })
    })?;
    fx.advance(Duration::ZERO)?;
    fx.cx.update_window(fx.window, |_, window, cx| {
        window.dispatch_action(Box::new(ui::OpenPullRequests), cx);
    })?;
    fx.wait("recorded GitHub identity", |fx| {
        fx.read(|root, cx| {
            root.pull_requests_page()
                .is_some_and(|page| page.read(cx).account().is_some())
        })
    })?;
    let page = fx.read(|root, _| root.pull_requests_page().unwrap().clone());
    fx.cx.update_window(fx.window, |_, window, cx| {
        page.focus_handle(cx).focus(window, cx)
    })?;
    fx.cx.run_until_parked();
    fx.type_text("repo:i098/codex-gpui")?;
    fx.advance(Duration::from_millis(250))?;
    wait_for_open_queries(&mut fx, &page)?;
    assert!(
        (fx.read(|_, cx| page.read(cx).detail_width()) - 723.86584).abs() < 0.0001,
        "The initial ratio must retain the preceding Home inset",
    );
    if capture_pane(&mut fx, "open-all", reset_state)? {
        return Ok(());
    }
    page.update(&mut fx.cx, |page, cx| {
        page.view = View::Authored;
        page.lifecycle = "merged".into();
        cx.emit(Event::Query);
    });
    fx.wait("first recorded merged page", |fx| {
        fx.read(|_, cx| {
            let page = page.read(cx);
            page.streams.len() == 1
                && page.streams[0].loaded
                && !page.streams[0].loading
                && page.streams[0].cursor.is_some()
        })
    })?;
    let first_ids = fx.read(|_, cx| {
        page.read(cx).streams[0]
            .items
            .iter()
            .map(|item| item.node_id.clone())
            .collect::<Vec<_>>()
    });
    if capture_pane(&mut fx, "merged-first-page", reset_state)? {
        return Ok(());
    }
    page.update(&mut fx.cx, |_, cx| cx.emit(Event::More));
    fx.wait("second recorded merged page", |fx| {
        fx.read(|_, cx| {
            let stream = &page.read(cx).streams[0];
            !stream.loading
                && stream
                    .items
                    .iter()
                    .any(|item| !first_ids.contains(&item.node_id))
        })
    })?;
    let all_ids = fx.read(|_, cx| {
        page.read(cx).streams[0]
            .items
            .iter()
            .map(|item| item.node_id.clone())
            .collect::<Vec<_>>()
    });
    assert_eq!(&all_ids[..first_ids.len()], first_ids);
    assert_eq!(all_ids.iter().collect::<HashSet<_>>().len(), all_ids.len());
    if capture_pane(&mut fx, "merged-second-page", reset_state)? {
        return Ok(());
    }
    let selected = fx.read(|_, cx| page.read(cx).streams[0].items[0].pull_request.clone());
    page.update(&mut fx.cx, |_, cx| {
        cx.emit(Event::Select(Some(selected.clone())))
    });
    fx.wait("recorded detail response", |fx| {
        fx.read(|_, cx| matches!(page.read(cx).detail, Some(Ok(_))))
    })?;
    assert_eq!(
        fx.read(|_, cx| page.read(cx).detail_width()),
        724.,
        "A fresh detail mount must reach its width before reset",
    );
    if capture_pane(&mut fx, "merged-detail", reset_state)? {
        return Ok(());
    }
    if cfg!(target_os = "macos") {
        fx.click((690., 67.))?;
    } else {
        page.update(&mut fx.cx, |page, cx| {
            page.set_detail_tab(DetailTab::Code, cx)
        });
    }
    fx.wait("patch failure", |fx| {
        fx.read(|_, cx| matches!(page.read(cx).code, Some(Err(_))))
    })?;
    assert!(fx.read(|_, cx| matches!(page.read(cx).detail, Some(Ok(_)))));
    assert_eq!(
        fx.read(|_, cx| page.read(cx).selected.clone()),
        Some(selected)
    );
    host.fail_code.store(false, Ordering::SeqCst);
    page.update(&mut fx.cx, |_, cx| cx.emit(Event::Code));
    fx.wait("patch retry", |fx| {
        fx.read(|_, cx| matches!(page.read(cx).code, Some(Ok(_))))
    })?;
    fx.read(|_, cx| {
        let panel = page
            .read(cx)
            .code
            .as_ref()
            .unwrap()
            .as_ref()
            .unwrap()
            .read(cx);
        assert_eq!(panel.files.len(), 1);
        assert_eq!(panel.files[0].name.as_ref(), "example.rs");
        assert_eq!((panel.files[0].added, panel.files[0].removed), (1, 1));
    });
    capture(&mut fx, "merged-code")?;
    page.update(&mut fx.cx, |page, cx| {
        page.set_detail_tab(DetailTab::Summary, cx)
    });
    if cfg!(target_os = "macos") {
        let default_pane = fx.reference_shot()?;
        let original_fullscreen = fx
            .cx
            .update_window(fx.window, |_, window, _| window.is_fullscreen())?;
        fx.click((1246., 67.))?;
        capture(&mut fx, "detail-expanded")?;
        assert_eq!(
            fx.cx
                .update_window(fx.window, |_, window, _| window.is_fullscreen())?,
            original_fullscreen,
            "The workspace control must not change OS fullscreen state",
        );
        fx.click((1246., 67.))?;
        let restored_pane = fx.reference_shot()?;
        assert!(
            default_pane.as_raw() == restored_pane.as_raw(),
            "Restoring the workspace pane must restore its default pixels",
        );
        capture(&mut fx, "detail-restored")?;
    }
    let divider = 1276. - fx.read(|_, cx| page.read(cx).detail_width());
    fx.drag((divider, 300.), (divider + 50., 300.), 4)?;
    assert!((fx.read(|_, cx| page.read(cx).detail_width()) - 674.).abs() < 0.001);
    capture(&mut fx, "detail-dragged")?;
    reset_pane(&mut fx)?;
    assert_eq!(fx.read(|_, cx| page.read(cx).detail_width()), 724.);
    capture(&mut fx, "detail-reset-after-drag")?;
    fx.resize(1440., 832.)?;
    fx.advance(Duration::ZERO)?;
    fx.advance(Duration::ZERO)?;
    let expected = 320. + (724. - 320.) / (1224. - 672.) * (1384. - 672.);
    assert!((fx.read(|_, cx| page.read(cx).detail_width()) - expected).abs() < 0.001);
    capture(&mut fx, "detail-resized")?;
    page.update(&mut fx.cx, |page, cx| page.reset_detail_width(cx));
    assert_eq!(fx.read(|_, cx| page.read(cx).detail_width()), 724.);
    capture(&mut fx, "detail-resized-reset")?;
    host.switched.store(true, Ordering::SeqCst);
    page.update(&mut fx.cx, |_, cx| cx.emit(Event::Retry));
    fx.wait("switched GitHub account", |fx| {
        fx.read(|_, cx| {
            page.read(cx)
                .account()
                .is_some_and(|account| account.login == "changed-account")
        })
    })?;
    Ok(())
}

fn wait_for_open_queries(
    fx: &mut Fx,
    page: &gpui::Entity<ui::screens::pull_requests::Page>,
) -> Result<()> {
    let loaded = fx.wait("recorded open queries", |fx| {
        fx.read(|_, cx| {
            let page = page.read(cx);
            !page.streams.is_empty()
                && page
                    .streams
                    .iter()
                    .all(|stream| stream.loaded && !stream.loading && stream.error.is_none())
        })
    });
    if let Err(error) = loaded {
        let (query, states) = fx.read(|_, cx| {
            let page = page.read(cx);
            (
                page.query.clone(),
                page.streams
                    .iter()
                    .map(|stream| {
                        (
                            stream.key,
                            stream.loaded,
                            stream.loading,
                            stream.error.clone(),
                        )
                    })
                    .collect::<Vec<_>>(),
            )
        });
        capture(fx, "open-query-error")?;
        anyhow::bail!("{error:#}; query={query:?}; states={states:?}");
    }
    Ok(())
}

#[test]
fn pane_mount_reads_the_preceding_shell_after_resize_and_sidebar_changes() -> Result<()> {
    for (index, (size, sidebar_open)) in [
        ((1280., 832.), true),
        ((1440., 700.), false),
        ((1100., 900.), true),
    ]
    .into_iter()
    .enumerate()
    {
        let mut fx = Fx::with("dark", size, None, |_| {})?;
        assert!(fx.sidebar_open());
        if !sidebar_open {
            fx.keys("cmd-b")?;
            fx.advance(Duration::from_millis(500))?;
        }
        assert_eq!(fx.sidebar_open(), sidebar_open);
        for visit in 0..2 {
            if visit != 0 {
                fx.cx.update_window(fx.window, |_, window, cx| {
                    window.dispatch_action(Box::new(ui::NewChat), cx);
                })?;
                assert_eq!(fx.sidebar_open(), sidebar_open);
                fx.keys("cmd-b")?;
                fx.advance(Duration::from_millis(500))?;
                assert_eq!(fx.sidebar_open(), !sidebar_open);
            }
            assert_eq!(fx.sidebar_open(), sidebar_open != (visit != 0));
            fx.advance(Duration::ZERO)?;
            let inset = f64::from(f32::from(
                fx.shell(|shell| shell.main_content_inline_inset()),
            ));
            assert_eq!(inset > 0., sidebar_open != (visit != 0));
            let viewport = (size.0 + 100. * (visit + 1) as f32, size.1 - 40.);
            fx.resize(viewport.0, viewport.1)?;
            fx.advance(Duration::ZERO)?;
            let available = f64::from(viewport.0 - 56.);
            let initial_available = available - inset;
            let default = (f64::from(viewport.1) * 1.6)
                .min(initial_available - 500.)
                .max(640_f64.min(initial_available - 352.))
                .max(320.);
            let ratio = ((default - 320.) / (initial_available - 672.)).clamp(0., 1.);
            fx.cx.update_window(fx.window, |_, window, cx| {
                window.dispatch_action(Box::new(ui::OpenPullRequests), cx);
            })?;
            fx.advance(Duration::ZERO)?;
            fx.advance(Duration::ZERO)?;
            let width =
                fx.read(|root, cx| root.pull_requests_page().unwrap().read(cx).detail_width());
            assert!((f64::from(width) - (320. + ratio * (available - 672.))).abs() < 0.001);
            capture(&mut fx, &format!("navigation-resized-{index}-{visit}"))?;
        }
    }
    Ok(())
}

fn reset_pane(fx: &mut Fx) -> Result<()> {
    let width = fx.read(|root, cx| root.pull_requests_page().unwrap().read(cx).detail_width());
    let position = fx.cx.update_window(fx.window, |_, window, _| {
        gpui::point(
            window.viewport_size().width - gpui::px(4. + width),
            gpui::px(300.),
        )
    })?;
    fx.hover((position.x.into(), position.y.into()))?;
    fx.input(gpui::PlatformInput::MouseDown(gpui::MouseDownEvent {
        button: gpui::MouseButton::Left,
        position,
        modifiers: gpui::Modifiers::default(),
        click_count: 2,
        first_mouse: false,
    }))?;
    fx.input(gpui::PlatformInput::MouseUp(gpui::MouseUpEvent {
        button: gpui::MouseButton::Left,
        position,
        modifiers: gpui::Modifiers::default(),
        click_count: 2,
    }))
}

fn capture_pane(fx: &mut Fx, state: &str, reset_state: Option<&str>) -> Result<bool> {
    if reset_state.is_none() {
        capture(fx, state)?;
        return Ok(false);
    }
    if reset_state != Some(state) {
        return Ok(false);
    }
    let width = fx.read(|root, cx| root.pull_requests_page().unwrap().read(cx).detail_width());
    let expected_initial = if state == "merged-detail" {
        724.
    } else {
        723.86584
    };
    assert!((width - expected_initial).abs() < 0.0001);
    capture(fx, &format!("{state}-before-reset"))?;
    reset_pane(fx)?;
    let expected = if state == "merged-detail" {
        724.
    } else {
        723.5
    };
    assert_eq!(
        fx.read(|root, cx| root.pull_requests_page().unwrap().read(cx).detail_width()),
        expected,
    );
    capture(fx, &format!("{state}-reset"))?;
    Ok(true)
}

fn capture(fx: &mut Fx, name: &str) -> Result<()> {
    if let Some(directory) = std::env::var_os("CGX_PULL_REQUEST_EVIDENCE") {
        std::fs::create_dir_all(&directory)?;
        fx.reference_shot()?
            .save(std::path::Path::new(&directory).join(format!("{name}.png")))?;
        let width = fx.read(|root, cx| {
            root.pull_requests_page()
                .map(|page| page.read(cx).detail_width())
        });
        let viewport = fx.cx.update_window(fx.window, |_, window, _| {
            let size = window.viewport_size();
            [
                f32::from(size.width),
                f32::from(size.height),
                window.scale_factor(),
            ]
        })?;
        std::fs::write(
            std::path::Path::new(&directory).join(format!("{name}.json")),
            serde_json::to_vec_pretty(&serde_json::json!({
                "state": name,
                "width": width,
                "viewport": viewport,
            }))?,
        )?;
    }
    Ok(())
}
