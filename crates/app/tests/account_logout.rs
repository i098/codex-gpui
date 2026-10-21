#![cfg(target_os = "macos")]

#[path = "common/mod.rs"]
mod common;

use anyhow::{Context as _, Result, ensure};
use app::Route;
use common::{Fx, GREETING, exchange, script};
use serde_json::json;

#[test]
fn proxy() {
    common::proxy_child();
}

fn open_logout(fx: &mut Fx) -> Result<()> {
    fx.keys("cmd-k")?;
    fx.type_text("Log out")?;
    fx.keys("enter")?;
    ensure!(fx.read(|root, _| root.command_menu().is_none()));
    Ok(())
}

fn shot(fx: &mut Fx, name: &str) -> Result<()> {
    if let Some(directory) = std::env::var_os("CGX_LOGOUT_SHOTS") {
        let directory = std::path::PathBuf::from(directory);
        std::fs::create_dir_all(&directory)?;
        fx.shot()?.save(directory.join(format!("{name}.png")))?;
    }
    Ok(())
}

/// Bounding box `[left, top, right, bottom]`, in 1280-wide points, of the recorded focus ring
/// (`rgb(58, 131, 247)`) inside the dialog area, or `None` when no pixel has that colour.
fn ring_box(fx: &mut Fx) -> Result<Option<[f32; 4]>> {
    let image = fx.shot()?;
    let scale = image.width() as f32 / 1280.;
    let mut found: Option<[u32; 4]> = None;
    for y in (320. * scale) as u32..(500. * scale) as u32 {
        for x in (440. * scale) as u32..(840. * scale) as u32 {
            if image.get_pixel(x, y).0[..3] == [58, 131, 247] {
                let [l, t, r, b] = found.unwrap_or([x, y, x, y]);
                found = Some([l.min(x), t.min(y), r.max(x + 1), b.max(y + 1)]);
            }
        }
    }
    Ok(found.map(|b| b.map(|v| v as f32 / scale)))
}

/// The original `Log out?` card is 380 × 170 with the `Signed in as` line, centered in the window;
/// its 32 px buttons sit 20 px from the bottom and right edges (`refs/interactions/logout.json`),
/// so the 2 px ring around a button is 36 high and spans y 447 to 483, as recorded.
fn ring_fits(ring: [f32; 4]) -> bool {
    let card_bottom = 832. / 2. + 170. / 2.;
    (ring[3] - ring[1] - 36.).abs() <= 0.6 && (ring[3] - (card_bottom - 18.)).abs() <= 0.6
}

#[test]
fn logout_requires_confirmation_and_preserves_the_session_on_failure() -> Result<()> {
    let mut lines = script(None, |line| {
        if line["dir"] == "send" && line["msg"]["method"] == "thread/list" {
            line["msg"]["params"] = json!({});
        }
    })?;
    let mut failed = exchange("account/logout", json!({}));
    failed[1]["msg"].as_object_mut().unwrap().remove("result");
    failed[1]["msg"]["error"] = json!({"code": -32603, "message": "Replay logout failure"});
    lines.extend(failed);
    lines.extend(exchange("account/logout", json!({})));
    lines.extend(exchange(
        "account/read",
        json!({"account": {"type": "chatgpt", "email": "user@example.com", "planType": "promax"}, "requiresOpenaiAuth": true}),
    ));
    lines.extend(exchange(
        "configRequirements/read",
        json!({"requirements":{"allowedLoginMethods":["api"]}}),
    ));
    lines.extend(exchange("account/login/start", json!({"type":"apiKey"})));
    let mut new_thread = common::recorded_thread(GREETING)?;
    new_thread["turns"] = json!([]);
    let pages: Vec<_> = (0..11)
        .map(|index| {
            let mut thread = new_thread.clone();
            thread["id"] = format!("after-login-{index}").into();
            thread
        })
        .collect();
    lines.extend(exchange(
        "thread/list",
        json!({"data": &pages[..10], "nextCursor": "after-login-page"}),
    ));
    let mut next_page = exchange(
        "thread/list",
        json!({"data": &pages[10..], "nextCursor": null}),
    );
    next_page[0]["msg"]["params"] = json!({"cursor": "after-login-page", "limit": 10});
    lines.extend(next_page);
    new_thread["id"] = "after-login-new-chat".into();
    lines.extend(exchange(
        "thread/start",
        json!({
            "thread": new_thread,
            "model": "gpt-6-astra",
            "cwd": common::CWD,
            "reasoningEffort": "low",
        }),
    ));
    lines.extend(exchange("turn/start", json!({})));
    lines.extend(exchange("turn/start", json!({})));
    let theme = match std::env::var("CGX_LOGOUT_THEME").as_deref() {
        Ok("light") => "light",
        _ => "dark",
    };
    let mut fx = Fx::captured(theme, (1280., 832.), lines)?;
    let home = tempfile::tempdir()?;
    fx.root.update(&mut fx.cx, |root, cx| {
        root.set_host(
            std::sync::Arc::new(app::host::LocalHost::new(home.path())),
            cx,
        );
    });
    let home_composer = fx.composer();
    fx.keys("cmd-1")?;
    fx.wait_thread(GREETING)?;
    fx.type_text("Keep this draft")?;
    let requests = fx.sent()?;
    open_logout(&mut fx)?;
    shot(&mut fx, "confirmation")?;
    let cancel = ring_box(&mut fx)?.context("Cancel ring")?;
    ensure!(
        ring_fits(cancel),
        "Cancel must be ringed and 20 px above the card bottom when the dialog opens"
    );
    ensure!(
        fx.read(|root, _| root.dialog().is_some()),
        "Log out must open confirmation"
    );
    fx.keys("enter")?;
    ensure!(fx.read(|root, _| root.dialog().is_none() && !root.signed_out()));
    ensure!(fx.sent()? == requests, "Cancel must send nothing");
    ensure!(fx.route() == Route::Thread(GREETING.into()));
    ensure!(fx.composer_text()? == "Keep this draft");

    open_logout(&mut fx)?;
    fx.keys("escape")?;
    ensure!(fx.sent()? == requests, "Escape must send nothing");
    fx.click((26., 806.))?;
    fx.click((118., 672.))?;
    ensure!(fx.read(|root, _| root.dialog().is_some()));
    fx.keys("tab")?;
    shot(&mut fx, "keyboard-focus")?;
    let logout = ring_box(&mut fx)?.context("Log out ring")?;
    ensure!(
        ring_fits(logout) && (logout[2] - 812.).abs() <= 0.6,
        "Tab must ring Log out, 20 px from the card's right edge"
    );
    ensure!(
        cancel[1] == logout[1] && cancel[3] == logout[3],
        "Cancel {cancel:?} and Log out {logout:?} must share top and bottom"
    );
    fx.keys("enter")?;
    fx.wait("logout failure", |fx| {
        !fx.read(|root, cx| root.toasts(cx).is_empty())
    })?;
    ensure!(!fx.read(|root, _| root.signed_out()));
    ensure!(fx.thread_id().as_deref() == Some(GREETING));
    ensure!(fx.composer_focused()?);
    fx.type_text(" after failure")?;
    ensure!(fx.composer_text()? == "Keep this draft after failure");
    shot(&mut fx, "failure")?;
    let image = home.path().join("previous-account.png");
    image::RgbaImage::from_pixel(1, 1, image::Rgba([255, 0, 0, 255])).save(&image)?;
    for composer in [home_composer, fx.composer()] {
        composer.update(&mut fx.cx, |composer, cx| {
            composer.drop_files(std::slice::from_ref(&image), cx);
        });
    }

    fx.menu("Codex", "Log Out")?;
    ensure!(fx.read(|root, _| root.dialog().is_some()));
    fx.keys("tab")?;
    fx.keys("enter")?;
    fx.wait("login after logout", |fx| {
        fx.read(|root, _| root.signed_out())
    })?;
    fx.wait("managed sign-in policy", |fx| {
        fx.read(|root, cx| {
            root.login().is_some_and(|login| {
                login.read(cx).policy()
                    == ui::screens::signin::policy::Policy::Ready {
                        api: true,
                        chatgpt: false,
                    }
            })
        })
    })?;
    ensure!(fx.read(|root, _| root.thread().is_none() && root.recents().is_empty()));
    ensure!(fx.route() == Route::Home);
    shot(&mut fx, "success")?;
    let requests = fx.sent()?;
    let logout: Vec<_> = requests
        .iter()
        .filter(|r| r["method"] == "account/logout")
        .map(|r| r["params"].clone())
        .collect();
    ensure!(logout == [json!({}), json!({})]);
    let late_thread = common::recorded_thread(GREETING)?;
    fx.root.update(&mut fx.cx, |root, cx| {
        root.on_event(
            bridge::Event::Notification {
                method: "thread/started".into(),
                params: json!({"thread": late_thread}),
            },
            cx,
        );
    });
    ensure!(fx.read(|root, _| root.thread().is_none() && root.recents().is_empty()));
    for key in ["cmd-[", "cmd-1", "cmd-k", "cmd-n"] {
        fx.keys(key)?;
    }
    ensure!(fx.read(|root, _| root.signed_out() && root.command_menu().is_none()));
    ensure!(fx.route() == Route::Home && fx.sent()? == requests);
    fx.keys("tab tab enter")?;
    fx.type_text("sk-local-test")?;
    fx.keys("enter")?;
    fx.wait("first signed-in chat page", |fx| {
        fx.read(|root, _| root.recents().len() >= 10)
    })?;
    let sidebar = fx.read(|root, _| root.sidebar().clone());
    sidebar.update(&mut fx.cx, |_, cx| {
        cx.emit(ui::screens::sidebar::LoadMore);
    });
    fx.wait("remaining signed-in chats", |fx| {
        fx.read(|root, _| root.recents().contains(&"after-login-10".into()))
    })?;
    fx.keys("cmd-n")?;
    fx.type_text("New account home message")?;
    fx.keys("enter")?;
    fx.wait_thread("after-login-new-chat")?;
    fx.type_text("New account thread message")?;
    fx.keys("enter")?;
    fx.wait("new account sends", |fx| {
        fx.sent().is_ok_and(|requests| {
            requests
                .iter()
                .filter(|r| r["method"] == "turn/start")
                .count()
                == 2
        })
    })?;
    let inputs: Vec<_> = fx
        .sent()?
        .into_iter()
        .filter(|r| r["method"] == "turn/start")
        .map(|r| r["params"]["input"].clone())
        .collect();
    ensure!(
        inputs
            == [
                json!([{"type": "text", "text": "New account home message", "text_elements": []}]),
                json!([{"type": "text", "text": "New account thread message", "text_elements": []}]),
            ],
        "Previous account attachments reached a new submission: {inputs:?}"
    );
    // The signed-in email is read again after sign-in: the card keeps its `Signed in as` line.
    ensure!(
        fx.sent()?
            .iter()
            .filter(|r| r["method"] == "account/read")
            .count()
            == 2,
        "account/read must run at start and after sign-in"
    );
    fx.menu("Codex", "Log Out")?;
    ensure!(
        ring_box(&mut fx)?.is_some_and(ring_fits),
        "the new session's logout card lost its email line"
    );
    shot(&mut fx, "confirmation-new-session")?;
    Ok(())
}

mod signin_checks {
    use super::common;
    use anyhow::{Result, ensure};
    use common::{Fx, exchange, script};
    use serde_json::{Value, json};
    use ui::screens::signin::policy::Policy;
    fn signed_out(line: &mut Value) {
        if line["dir"] == "recv" && line["msg"]["result"].get("requiresOpenaiAuth").is_some() {
            line["msg"]["result"] = json!({"account":null,"requiresOpenaiAuth":true});
        }
        if line["dir"] == "send" && line["msg"]["method"] == "thread/list" {
            line["msg"]["params"] = json!({});
        }
    }

    fn policy(fx: &Fx) -> Option<Policy> {
        fx.read(|root, cx| root.login().map(|login| login.read(cx).policy()))
    }

    fn shot(fx: &mut Fx, name: &str) -> Result<()> {
        if let Some(directory) = std::env::var_os("CGX_SIGNIN_SHOTS") {
            let directory = std::path::PathBuf::from(directory);
            std::fs::create_dir_all(&directory)?;
            fx.shot()?.save(directory.join(format!("{name}.png")))?;
        }
        Ok(())
    }

    #[test]
    fn managed_signin_restrictions_hide_methods_and_api_paste_submits_once() -> Result<()> {
        common::install_panic_hook();
        for (requirements, expected, name) in [
            (
                json!({"allowedLoginMethods":["api","chatgpt"]}),
                Policy::Ready {
                    api: true,
                    chatgpt: true,
                },
                "login-main",
            ),
            (json!({}), Policy::Unsupported, "requirements-old-cli"),
            (
                json!({"allowedLoginMethods":[]}),
                Policy::Ready {
                    api: false,
                    chatgpt: false,
                },
                "requirements-disabled",
            ),
            (
                json!({"allowedLoginMethods":["unknown"]}),
                Policy::Error,
                "requirements-error",
            ),
            (
                json!({"allowedLoginMethods":["api"]}),
                Policy::Ready {
                    api: true,
                    chatgpt: false,
                },
                "requirements-api-only",
            ),
        ] {
            let mut lines = script(None, signed_out)?;
            lines.extend(exchange(
                "configRequirements/read",
                json!({"requirements":requirements}),
            ));
            let mut refused = exchange("account/login/start", Value::Null);
            refused[1]["msg"].as_object_mut().unwrap().remove("result");
            refused[1]["msg"]["error"] = json!({"code":-32000,"message":"Invalid fixture API key"});
            lines.extend(refused);
            let mut fx = Fx::captured("dark", (1280., 832.), lines)?;
            fx.wait("managed sign-in policy", |fx| policy(fx) == Some(expected))?;
            shot(&mut fx, name)?;
            if expected
                != (Policy::Ready {
                    api: true,
                    chatgpt: false,
                })
            {
                continue;
            }
            fx.keys("tab tab enter")?;
            let field = fx.read(|root, cx| root.login().unwrap().read(cx).api_key_field().clone());
            fx.cx.update(|cx| {
                cx.write_to_clipboard(gpui::ClipboardItem::new_string(
                    "  sk-fixture-rejected  ".into(),
                ))
            });
            fx.keys("cmd-v")?;
            ensure!(field.read_with(&fx.cx, |field, _| field.field_text()
                == "  sk-fixture-rejected  "));
            shot(&mut fx, "api-key-paste")?;
            fx.keys("enter")?;
            fx.wait("API error", |fx| {
                fx.read(|root, cx| {
                    root.login()
                        .is_some_and(|login| login.read(cx).error().is_some())
                })
            })?;
            shot(&mut fx, "api-key-error")?;
            let sent = fx.sent()?;
            let attempts: Vec<_> = sent
                .iter()
                .filter(|message| message["method"] == "account/login/start")
                .collect();
            ensure!(attempts.len() == 1);
            ensure!(
                attempts[0]["params"] == json!({"type":"apiKey","apiKey":"sk-fixture-rejected"})
            );
            fx.root.update(&mut fx.cx, |root, cx| {
                root.on_event(
                    bridge::Event::Notification {
                        method: "account/login/completed".into(),
                        params: json!({"loginId":"unrelated","success":true}),
                    },
                    cx,
                )
            });
            ensure!(fx.read(|root, _| root.signed_out()));
        }
        Ok(())
    }

    struct OnboardingHost {
        context: Value,
        writes: std::sync::Mutex<Vec<Value>>,
    }

    impl app::host::Host for OnboardingHost {
        fn fetch(&self, request: &app::host::Request) -> Result<Value> {
            if let app::host::Request::Http(request) = request {
                if request["url"] == "/wham/onboarding/context" {
                    return Ok(self.context.clone());
                }
                if request["url"] == "/wham/onboarding/desktop/complete" {
                    self.writes.lock().unwrap().push(request.clone());
                    anyhow::bail!("Offline completion is denied");
                }
            }
            anyhow::bail!("No recorded response for this onboarding request")
        }
        fn projects(&self) -> Result<Vec<ui::screens::sidebar::Project>> {
            Ok(Vec::new())
        }
    }

    #[test]
    fn onboarding_keeps_failed_saves_and_cancelled_skips_on_the_page() -> Result<()> {
        common::install_panic_hook();
        let original: Value = serde_json::from_str(&std::fs::read_to_string(format!(
            "{}/tools/route1/oracle.json",
            common::ROOT
        ))?)?;
        let context = original
            .as_object()
            .unwrap()
            .iter()
            .find(|(key, _)| key.contains("/wham/onboarding/context"))
            .unwrap()
            .1
            .clone();
        let host = std::sync::Arc::new(OnboardingHost {
            context,
            writes: Default::default(),
        });
        let mut fx = Fx::new()?;
        fx.root
            .update(&mut fx.cx, |root, cx| root.set_host(host.clone(), cx));
        let root = fx.root.clone();
        fx.cx.update_window(fx.window, |_, window, cx| {
            root.update(cx, |root, cx| {
                root.show_onboarding(ui::screens::onboarding::Page::Welcome, window, cx)
            })
        })?;
        fx.wait("onboarding context", |fx| {
            fx.read(|root, cx| {
                root.onboarding()
                    .is_some_and(|page| !page.read(cx).loading())
            })
        })?;
        shot(&mut fx, "onboarding-role")?;
        for _ in 0..14 {
            fx.keys("tab")?;
        }
        fx.keys("enter")?;
        fx.wait("onboarding save error", |fx| {
            fx.read(|root, cx| {
                root.onboarding()
                    .is_some_and(|page| page.read(cx).error().is_some())
            })
        })?;
        ensure!(fx.route() == app::Route::Onboarding(ui::screens::onboarding::Page::Welcome));
        ensure!(host.writes.lock().unwrap().len() == 1);
        shot(&mut fx, "onboarding-save-error")?;
        fx.cx.update_window(fx.window, |_, window, cx| {
            root.update(cx, |root, cx| {
                root.show_onboarding(ui::screens::onboarding::Page::Onboarding, window, cx)
            })
        })?;
        fx.wait("second onboarding context", |fx| {
            fx.read(|root, cx| {
                root.onboarding()
                    .is_some_and(|page| !page.read(cx).loading())
            })
        })?;
        for _ in 0..15 {
            fx.keys("tab")?;
        }
        fx.keys("enter")?;
        ensure!(fx.read(|root, cx| root.onboarding().unwrap().read(cx).confirming_skip()));
        shot(&mut fx, "onboarding-skip-dialog")?;
        fx.keys("escape")?;
        ensure!(!fx.read(|root, cx| root.onboarding().unwrap().read(cx).confirming_skip()));
        ensure!(host.writes.lock().unwrap().len() == 1);
        Ok(())
    }
}
