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
    anyhow::ensure!(arguments.len() == 3, "usage: licenses INPUT OUTPUT");
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
    let window = cx.open_window(size(px(900.), px(650.)), |_, cx| {
        let page = ui::screens::settings::subtrees::licenses(
            async move {
                ready.store(true, Ordering::Release);
                Ok(Some(contents))
            },
            cx,
        );
        cx.new(|_| Preview(page.into()))
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
