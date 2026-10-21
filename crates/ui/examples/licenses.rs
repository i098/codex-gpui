use gpui::{
    AnyView, AppContext as _, Context, HeadlessAppContext, IntoElement, ParentElement, Render,
    Styled, Window, div, px, size,
};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
use theme::{ActiveTheme as _, Theme};

struct Preview(AnyView);

impl Render for Preview {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .bg(cx.theme().surface)
            .font_family(ui::SANS)
            .child(self.0.clone())
    }
}

fn main() -> anyhow::Result<()> {
    std::panic::set_hook(Box::new(|info| {
        eprintln!("{info}");
        std::process::exit(101);
    }));
    std::thread::spawn(|| {
        std::thread::sleep(Duration::from_secs(30));
        eprintln!("license probe exceeded 30 seconds");
        std::process::exit(124);
    });
    let arguments: Vec<_> = std::env::args().collect();
    anyhow::ensure!(
        arguments.len() == 3 || arguments.len() == 4 && arguments[3] == "shell",
        "usage: licenses INPUT OUTPUT [shell]"
    );
    let full_shell = arguments.get(3).is_some();
    let contents = std::fs::read_to_string(&arguments[1])?;
    let bytes = contents.len();
    let loaded = Arc::new(AtomicBool::new(false));
    let ready = loaded.clone();
    let mut cx = HeadlessAppContext::with_platform(
        textsys::text_system(),
        Arc::new(theme::Assets),
        gpui_platform::current_headless_renderer,
    );
    cx.update(|cx| {
        cx.set_global(Theme::dark());
        theme::load_fonts(cx);
    });
    let started = Instant::now();
    let viewport = if full_shell {
        size(px(1280.), px(832.))
    } else {
        size(px(900.), px(650.))
    };
    let window = cx.open_window(viewport, |_, cx| {
        let page = ui::screens::settings::subtrees::licenses(
            async move {
                ready.store(true, Ordering::Release);
                Ok(Some(contents))
            },
            cx,
        );
        let view = if full_shell {
            let config: bridge::types::ConfigReadResponse =
                ui::fixture::response("readback.jsonl", "config/read", 0);
            let skills: serde_json::Value =
                ui::fixture::response("readback.jsonl", "skills/list", 0);
            let (shell, settings) = ui::screens::settings::shell(&config.config, &skills, None, cx);
            settings.update(cx, |settings, cx| settings.show_subtree(page.into(), cx));
            shell.update(cx, |shell, _| shell.can_go_back = true);
            shell.into()
        } else {
            page.into()
        };
        cx.new(|_| Preview(view))
    })?;
    while !loaded.load(Ordering::Acquire) {
        cx.run_until_parked();
        anyhow::ensure!(
            started.elapsed() < Duration::from_secs(30),
            "license load timed out"
        );
        std::thread::sleep(Duration::from_millis(1));
    }
    cx.run_until_parked();
    let first_frame_ms = started.elapsed().as_secs_f64() * 1000.;
    cx.capture_screenshot(window.into())?.save(&arguments[2])?;
    let started = Instant::now();
    cx.simulate_window_resize(window.into(), size(px(760.), px(650.)));
    cx.run_until_parked();
    let resize_frame_ms = started.elapsed().as_secs_f64() * 1000.;
    println!(
        "{{\"bytes\":{bytes},\"first_frame_ms\":{first_frame_ms:.3},\"resize_frame_ms\":{resize_frame_ms:.3}}}"
    );
    Ok(())
}
