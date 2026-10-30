use anyhow::{Result, ensure};
use app::{Root, Route, State};
use gpui::{AppContext as _, Focusable as _, HeadlessAppContext, Keystroke, px, size};
use std::{
    fs,
    sync::Arc,
    time::{Duration, Instant},
};

fn main() -> Result<()> {
    let directory = tempfile::Builder::new()
        .prefix("s08-")
        .tempdir_in(std::env::var("LIVE_ROOT")?)?;
    let cwd = directory.path().to_path_buf();
    let skill = cwd.join(".agents/skills/s08-proof/SKILL.md");
    fs::create_dir_all(skill.parent().unwrap())?;
    fs::write(
        &skill,
        "---\nname: s08-proof\ndescription: Verify live skill commands.\n---\nUse this skill to verify discovery.\n",
    )?;
    let mut command = bridge::default_command();
    command.current_dir(&cwd);
    let (bridge, events) = bridge::Bridge::spawn(command)?;
    #[cfg(target_os = "linux")]
    let renderer = || Ok(None);
    #[cfg(not(target_os = "linux"))]
    let renderer = gpui_platform::current_headless_renderer;
    let mut cx = HeadlessAppContext::with_platform(
        textsys::text_system(),
        Arc::new(theme::Assets),
        renderer,
    );
    cx.update(|cx| {
        theme::load_fonts(cx);
        cx.set_global(theme::Theme::dark());
        app::init(cx);
    });
    cx.allow_parking();
    let window = cx.open_window(size(px(1280.), px(832.)), |window, cx| {
        cx.new(|cx| {
            Root::new(
                State::default(),
                bridge,
                events,
                cwd.to_string_lossy().into_owned(),
                window,
                cx,
            )
        })
    })?;
    let root = cx.read_window(&window, |root, _| root)?;
    let wait =
        |cx: &mut HeadlessAppContext, ready: &dyn Fn(&HeadlessAppContext) -> bool| -> Result<()> {
            let start = Instant::now();
            loop {
                cx.run_until_parked();
                if ready(cx) {
                    return Ok(());
                }
                ensure!(
                    start.elapsed() < Duration::from_secs(60),
                    "live S08 flow timed out"
                );
                std::thread::sleep(Duration::from_millis(10));
            }
        };
    wait(&mut cx, &|cx| root.read_with(cx, |root, _| root.ready()))?;
    let type_text = |cx: &mut HeadlessAppContext, text: &str| -> Result<()> {
        cx.update_window(window.into(), |_, window, cx| {
            for character in text.chars() {
                window.dispatch_keystroke(
                    Keystroke {
                        key: character.to_string(),
                        ..Default::default()
                    },
                    cx,
                );
            }
        })?;
        cx.run_until_parked();
        Ok(())
    };
    let press = |cx: &mut HeadlessAppContext, key: &str| -> Result<()> {
        cx.update_window(window.into(), |_, window, cx| {
            window.dispatch_keystroke(Keystroke::parse(key).unwrap(), cx);
        })?;
        cx.run_until_parked();
        Ok(())
    };
    cx.update_window(window.into(), |_, window, cx| {
        root.update(cx, |root, cx| {
            root.composer(cx)
                .read(cx)
                .focus_handle(cx)
                .focus(window, cx)
        });
    })?;
    type_text(&mut cx, "/s08-proof")?;
    press(&mut cx, "tab")?;
    #[cfg(target_os = "macos")]
    cx.capture_screenshot(window.into())?
        .save(std::path::Path::new(&std::env::var("LIVE_ROOT")?).join("s08-skill.png"))?;
    let prompt = root.read_with(&cx, |root, cx| root.composer(cx).read(cx).prompt());
    ensure!(
        prompt.starts_with("[$s08-proof](")
            && prompt.ends_with("/.agents/skills/s08-proof/SKILL.md) "),
        "live skill invocation: {prompt}"
    );
    println!("skills/list: real workspace skill inserted as {prompt}");
    press(&mut cx, "cmd-a")?;
    press(&mut cx, "backspace")?;
    type_text(&mut cx, "/mcp")?;
    press(&mut cx, "enter")?;
    #[cfg(target_os = "macos")]
    cx.capture_screenshot(window.into())?
        .save(std::path::Path::new(&std::env::var("LIVE_ROOT")?).join("s08-mcp.png"))?;
    ensure!(root.read_with(&cx, |root, _| root.route() == &Route::Settings));
    println!("MCP: /mcp opened the existing settings route");
    Ok(())
}
