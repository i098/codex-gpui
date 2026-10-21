use gpui::{App, Global, Window};
use theme::{Appearance, Theme};
use ui::screens::settings::appearance::Preferences;

struct SystemMotion(bool);
impl Global for SystemMotion {}

struct DockIconState(String, bool);
impl Global for DockIconState {}

pub fn apply(mut theme: Theme, cx: &mut App) {
    let dark = theme.appearance == Appearance::Dark;
    let mut preferences = {
        let empty = serde_json::Map::new();
        let values = cx
            .try_global::<crate::host_settings::Settings>()
            .map_or(&empty, |settings| settings.read_all());
        Preferences::read(values, dark)
    };
    if let Some(previous) = cx
        .try_global::<Preferences>()
        .filter(|previous| previous.theme.fonts == preferences.theme.fonts)
    {
        preferences.ui_family = previous.ui_family.clone();
        preferences.code_family = previous.code_family.clone();
        preferences.content_family = previous.content_family.clone();
    } else if preferences.theme.fonts.ui.is_some()
        || preferences.theme.fonts.code.is_some()
        || preferences.theme.fonts.content.is_some()
    {
        let available = cx.text_system().all_font_names();
        let family = |value: &str, fallback: &str| {
            value
                .split(',')
                .map(|part| part.trim().trim_matches(['\'', '"']))
                .find(|name| {
                    available
                        .iter()
                        .any(|installed| installed.eq_ignore_ascii_case(name))
                })
                .unwrap_or(fallback)
                .to_owned()
                .into()
        };
        preferences.ui_family = family(&preferences.ui_family, ui::SANS);
        preferences.code_family = family(&preferences.code_family, ui::MONO);
        preferences.content_family = family(&preferences.content_family, ui::SANS);
        if let Some(face) = &preferences.theme.fonts.ui_face {
            preferences.ui_family = face.postscript_name.clone().into();
        }
        if let Some(face) = &preferences.theme.fonts.code_face {
            preferences.code_family = face.postscript_name.clone().into();
        }
        if let Some(face) = &preferences.theme.fonts.content_face {
            preferences.content_family = face.postscript_name.clone().into();
        }
    }
    if !cx.has_global::<SystemMotion>() {
        cx.set_global(SystemMotion(cx.reduce_motion()));
    }
    let reduced = match preferences.motion.as_str() {
        "on" => true,
        "off" => false,
        _ => cx.global::<SystemMotion>().0,
    };
    cx.set_reduce_motion(reduced);
    preferences.theme.apply(&mut theme);
    cx.set_global(preferences);
    cx.set_global(theme);
    cx.refresh_windows();
}

pub fn refresh(cx: &mut App) {
    let dark = cx.global::<Theme>().appearance == Appearance::Dark;
    apply(if dark { Theme::dark() } else { Theme::light() }, cx);
}

pub fn window(window: &mut Window, cx: &mut App) {
    let mut dock = None;
    if let Some(settings) = cx.try_global::<Preferences>() {
        window.set_rem_size(gpui::px(16. * settings.ui_size / 14.));
        window.set_pointer_cursors(settings.pointer);
        let dark = matches!(
            window.appearance(),
            gpui::WindowAppearance::Dark | gpui::WindowAppearance::VibrantDark
        );
        if !cx
            .try_global::<DockIconState>()
            .is_some_and(|state| state.0 == settings.dock && state.1 == dark)
        {
            dock = Some((settings.dock.clone(), dark));
        }
    }
    if let Some((choice, dark)) = dock {
        if let Err(error) = set_dock_icon(&choice, dark) {
            eprintln!("codex-gpui: could not set Dock icon: {error}");
        }
        cx.set_global(DockIconState(choice, dark));
    }
}

#[cfg(not(target_os = "macos"))]
fn set_dock_icon(_: &str, _: bool) -> anyhow::Result<()> {
    Ok(())
}

#[cfg(target_os = "macos")]
fn set_dock_icon(value: &str, dark: bool) -> anyhow::Result<()> {
    use anyhow::Context as _;
    use objc2::{
        AnyThread as _, msg_send,
        runtime::{AnyClass, AnyObject},
    };
    use objc2_app_kit::NSImage;
    use objc2_foundation::NSData;
    let bytes: &[u8] = match value {
        "codex-system" if dark => include_bytes!("../../../assets/app/dock-codexDark.png"),
        "codex-system" => include_bytes!("../../../assets/app/dock-codexLight.png"),
        _ => include_bytes!("../../../assets/app/dock-appDefault.png"),
    };
    let data = NSData::with_bytes(bytes);
    let image = NSImage::initWithData(NSImage::alloc(), &data).context("invalid Dock image")?;
    let class = AnyClass::get(c"NSApplication").context("NSApplication is unavailable")?;
    unsafe {
        let app: *mut AnyObject = msg_send![class, sharedApplication];
        let _: () = msg_send![app, setApplicationIconImage: &*image];
    }
    Ok(())
}

pub fn handle_change(
    root: &mut crate::Root,
    page: &gpui::Entity<ui::screens::settings::SettingsPage>,
    event: &ui::screens::settings::SettingChanged,
    cx: &mut gpui::Context<crate::Root>,
) -> bool {
    use ui::screens::settings::appearance::{COPY, PAIR, PairSaved};
    if event.path == COPY {
        if let Some(value) = event.value["value"].as_str() {
            cx.write_to_clipboard(gpui::ClipboardItem::new_string(value.to_owned()));
            root.toast(
                ui::toast::Toast::info(if event.value["dark"] == true {
                    "Dark theme copied"
                } else {
                    "Light theme copied"
                }),
                cx,
            );
        }
        return true;
    }
    if event.path != PAIR {
        return false;
    }
    let imported = event.value["imported"].as_bool().unwrap_or(false);
    let dark = event.value["dark"].as_bool().unwrap_or(false);
    let result = write_pair(&event.value, cx);
    let success = result.is_ok();
    if let Err(error) = result {
        eprintln!("codex-gpui: appearance write: {error:#}");
        root.toast(
            ui::toast::Toast::danger(if imported {
                "Couldn’t import theme"
            } else {
                "Couldn’t change theme"
            }),
            cx,
        );
    } else {
        refresh(cx);
        if imported {
            root.toast(
                ui::toast::Toast::info(if dark {
                    "Dark theme imported"
                } else {
                    "Light theme imported"
                }),
                cx,
            );
        }
    }
    let mut values = cx
        .try_global::<crate::host_settings::Settings>()
        .map(|settings| serde_json::Value::Object(settings.read_all().clone()))
        .unwrap_or_else(|| serde_json::json!({}));
    values["appearance-theme"] = serde_json::json!(root.state.theme);
    values["confirm-before-close"] = serde_json::json!(root.state.confirm_before_close);
    page.update(cx, |page, cx| {
        page.refresh_host(values, cx);
        cx.emit(PairSaved { imported, success });
    });
    true
}

fn write_pair(value: &serde_json::Value, cx: &mut App) -> anyhow::Result<()> {
    use anyhow::{Context as _, ensure};
    use ui::screens::settings::appearance::{ChromeTheme, PRESETS};
    let dark = value["dark"].as_bool().context("missing theme variant")?;
    let theme: ChromeTheme = serde_json::from_value(value["theme"].clone())?;
    ensure!(theme.valid(), "invalid theme");
    let preset = value["preset"].as_str().context("missing theme preset")?;
    ensure!(
        PRESETS.iter().any(|p| p.id == preset
            && if dark {
                p.dark.is_some()
            } else {
                p.light.is_some()
            }),
        "invalid theme preset"
    );
    ensure!(
        cx.has_global::<crate::host_settings::Settings>(),
        "host settings are unavailable"
    );
    cx.global_mut::<crate::host_settings::Settings>()
        .write_many([
            (
                if dark {
                    "appearanceDarkChromeTheme"
                } else {
                    "appearanceLightChromeTheme"
                },
                serde_json::json!(theme),
            ),
            (
                if dark {
                    "appearanceDarkCodeThemeId"
                } else {
                    "appearanceLightCodeThemeId"
                },
                serde_json::json!(preset),
            ),
        ])
}

#[cfg(all(test, target_os = "macos"))]
mod native_tests {
    use super::*;
    use anyhow::{Context as _, Result, ensure};
    use gpui::{AppContext as _, HeadlessAppContext, px, size};
    use serde_json::json;
    use std::{
        path::Path,
        sync::Arc,
        time::{Duration, Instant},
    };
    use ui::screens::settings::{
        SettingChanged,
        appearance::{ChromeTheme, PAIR},
    };

    fn click(
        cx: &mut HeadlessAppContext,
        window: gpui::AnyWindowHandle,
        x: f32,
        y: f32,
    ) -> Result<()> {
        let position = gpui::point(px(x), px(y));
        for event in [
            gpui::PlatformInput::MouseMove(gpui::MouseMoveEvent {
                position,
                pressed_button: None,
                modifiers: Default::default(),
            }),
            gpui::PlatformInput::MouseDown(gpui::MouseDownEvent {
                position,
                button: gpui::MouseButton::Left,
                modifiers: Default::default(),
                click_count: 1,
                first_mouse: false,
            }),
            gpui::PlatformInput::MouseUp(gpui::MouseUpEvent {
                position,
                button: gpui::MouseButton::Left,
                modifiers: Default::default(),
                click_count: 1,
            }),
        ] {
            cx.update_window(window, |_, window, cx| {
                window.dispatch_event(event, cx);
            })?;
            cx.run_until_parked();
        }
        Ok(())
    }

    fn key(cx: &mut HeadlessAppContext, window: gpui::AnyWindowHandle, value: &str) -> Result<()> {
        let key = gpui::Keystroke::parse(value)?;
        cx.update_window(window, |_, window, cx| {
            window.dispatch_keystroke(key, cx);
        })?;
        cx.run_until_parked();
        Ok(())
    }

    fn scroll(
        cx: &mut HeadlessAppContext,
        window: gpui::AnyWindowHandle,
        delta: f32,
    ) -> Result<()> {
        cx.update_window(window, |_, window, cx| {
            window.dispatch_event(
                gpui::PlatformInput::ScrollWheel(gpui::ScrollWheelEvent {
                    position: gpui::point(px(1100.), px(700.)),
                    delta: gpui::ScrollDelta::Pixels(gpui::point(px(0.), px(delta))),
                    modifiers: Default::default(),
                    touch_phase: gpui::TouchPhase::Moved,
                }),
                cx,
            );
        })?;
        cx.run_until_parked();
        Ok(())
    }

    fn capture(
        cx: &mut HeadlessAppContext,
        window: gpui::AnyWindowHandle,
        name: &str,
    ) -> Result<()> {
        if let Some(path) = std::env::var_os("CGX_APPEARANCE_SHOTS") {
            std::fs::create_dir_all(&path)?;
            cx.capture_screenshot(window)?
                .save(Path::new(&path).join(format!("{name}.png")))?;
        }
        Ok(())
    }
    #[test]
    fn appearance_changes_reach_the_native_window() -> Result<()> {
        std::panic::set_hook(Box::new(|info| {
            eprintln!("{info}");
            std::process::exit(101);
        }));
        let home = tempfile::tempdir()?;
        let mut settings =
            crate::host_settings::Settings::load(home.path().join("host-settings.json"))?;
        let mut recorded: serde_json::Value = serde_json::from_slice(&std::fs::read(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tools/route1/oracle.json"
        ))?)?;
        let history = recorded["persisted-atoms"]["state"]["seen-model-upgrade-list"].take();
        ensure!(
            history.is_array(),
            "original announcement history is missing"
        );
        settings.write("seen-model-upgrade-list", history)?;
        let (bridge, events) = bridge::Bridge::replay(Path::new(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../fixtures/readback.jsonl"
        )))?;
        let mut cx = HeadlessAppContext::with_platform(
            textsys::text_system(),
            Arc::new(theme::Assets),
            gpui_platform::current_headless_renderer,
        );
        cx.update(|cx| {
            theme::load_fonts(cx);
            cx.set_global(Theme::dark());
            cx.set_global(settings);
            crate::init(cx);
        });
        cx.allow_parking();
        let window = cx.open_window(size(px(1280.), px(832.)), |window, cx| {
            cx.new(|cx| {
                crate::Root::new(
                    crate::State {
                        theme: crate::ThemeSetting::Dark,
                        ..Default::default()
                    },
                    bridge,
                    events,
                    "/tmp/cgx-mac/proj".into(),
                    window,
                    cx,
                )
            })
        })?;
        let root = cx.read_window(&window, |root, _| root)?;
        cx.update(|cx| {
            root.update(cx, |root, cx| {
                root.set_host(Arc::new(crate::host::LocalHost::new(home.path())), cx)
            })
        });
        let deadline = Instant::now() + Duration::from_secs(30);
        while !root.read_with(&cx, |root, _| root.ready()) && Instant::now() < deadline {
            cx.run_until_parked();
            std::thread::sleep(Duration::from_millis(5));
        }
        ensure!(
            root.read_with(&cx, |root, _| root.ready()),
            "app-server fixture did not start"
        );
        cx.update_window(window.into(), |_, window, cx| {
            root.update(cx, |root, cx| {
                root.open_settings_section("Appearance", window, cx)
            })
        })?;
        cx.run_until_parked();
        capture(&mut cx, window.into(), "appearance-default")?;
        click(&mut cx, window.into(), 1067., 359.)?;
        capture(&mut cx, window.into(), "appearance-presets")?;
        click(&mut cx, window.into(), 1050., 430.)?;
        capture(&mut cx, window.into(), "appearance-codex-themes")?;
        key(&mut cx, window.into(), "escape")?;
        click(&mut cx, window.into(), 959., 359.)?;
        let copied = cx.update(|cx| cx.read_from_clipboard().and_then(|item| item.text()));
        ensure!(
            copied.as_deref().is_some_and(|value| {
                ui::screens::settings::appearance::import_string(value, true).is_ok()
            }),
            "Copy theme did not put a valid original share string on the clipboard"
        );
        click(&mut cx, window.into(), 931., 359.)?;
        capture(&mut cx, window.into(), "appearance-import")?;
        key(&mut cx, window.into(), "cmd-v")?;
        capture(&mut cx, window.into(), "appearance-import-filled")?;
        key(&mut cx, window.into(), "enter")?;
        cx.update(|cx| {
            let saved = cx.global::<crate::host_settings::Settings>().read_all();
            ensure!(
                saved.get("appearanceDarkChromeTheme").is_some(),
                "Import theme did not save the pasted theme"
            );
            ensure!(
                saved["appearanceDarkCodeThemeId"] == "codex",
                "Import theme did not preserve its preset"
            );
            Ok::<_, anyhow::Error>(())
        })?;
        cx.advance_clock(Duration::from_secs(4));
        cx.run_until_parked();
        click(&mut cx, window.into(), 1076., 464.)?;
        capture(&mut cx, window.into(), "appearance-color")?;
        click(&mut cx, window.into(), 800., 130.)?;
        click(&mut cx, window.into(), 488., 648.)?;
        capture(&mut cx, window.into(), "appearance-advanced")?;
        scroll(&mut cx, window.into(), -550.)?;
        capture(&mut cx, window.into(), "appearance-advanced-middle")?;
        scroll(&mut cx, window.into(), -550.)?;
        capture(&mut cx, window.into(), "appearance-advanced-bottom")?;
        let page = root
            .read_with(&cx, |root, _| root.settings_page.clone())
            .expect("appearance page");
        let mut theme = ChromeTheme::default_for(true);
        theme.surface = "#203040".into();
        theme.accent = "#e21d44".into();
        let face = textsys::font_faces("Menlo")
            .into_iter()
            .find(|face| face.style == "Bold")
            .context("native Menlo Bold face")?;
        theme.fonts.code = Some(face.family.clone());
        theme.fonts.content = Some("Arial".into());
        theme.fonts.code_face = Some(ui::screens::settings::appearance::FontFace {
            family: face.family,
            full_name: face.full_name,
            postscript_name: face.postscript_name,
        });
        cx.update(|cx| page.update(cx, |_, cx| {
            cx.emit(SettingChanged { path: PAIR, value: json!({"dark": true, "theme": theme, "preset": "github", "imported": true}) });
            cx.emit(SettingChanged { path: "host.sansFontSize", value: json!(16) });
            cx.emit(SettingChanged { path: "host.reduced-motion-preference", value: json!("on") });
        }));
        cx.run_until_parked();
        cx.update_window(window.into(), |_, window, cx| {
            ensure!(
                window.rem_size() == px(16. * 16. / 14.),
                "UI text scale did not reach the window"
            );
            ensure!(
                cx.reduce_motion(),
                "reduced motion did not reach the window"
            );
            ensure!(
                cx.global::<Theme>().surface == gpui::Hsla::from(gpui::rgb(0x203040)),
                "theme surface did not reach the window"
            );
            Ok::<_, anyhow::Error>(())
        })??;
        let saved = crate::host_settings::Settings::load(home.path().join("host-settings.json"))?;
        ensure!(
            saved.read_all()["appearanceDarkChromeTheme"]["surface"] == "#203040",
            "theme was not saved"
        );
        ensure!(
            saved.read_all()["appearanceDarkCodeThemeId"] == "github",
            "theme preset was not saved atomically"
        );
        if let Some(path) = std::env::var_os("CGX_APPEARANCE_SHOTS") {
            cx.capture_screenshot(window.into())?
                .save(Path::new(&path).join("appearance-custom.png"))?;
        }
        const THREAD: &str = "01a13de9-c48a-7b20-8382-db900881cb99";
        cx.update_window(window.into(), |_, window, cx| {
            root.update(cx, |root, cx| {
                root.navigate(crate::Route::Thread(THREAD.into()), window, cx);
            })
        })?;
        let deadline = Instant::now() + Duration::from_secs(30);
        while !root.read_with(&cx, |root, cx| {
            root.thread()
                .is_some_and(|thread| thread.read(cx).id() == THREAD)
        }) && Instant::now() < deadline
        {
            cx.run_until_parked();
            std::thread::sleep(Duration::from_millis(5));
        }
        ensure!(
            root.read_with(&cx, |root, cx| root
                .thread()
                .is_some_and(|thread| thread.read(cx).id() == THREAD)),
            "recorded thread did not load"
        );
        capture(&mut cx, window.into(), "appearance-thread-fonts")?;
        Ok(())
    }
}
