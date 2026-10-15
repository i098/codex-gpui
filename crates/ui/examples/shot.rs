//! Renders `ui::home_shell` headless at 1280x832 (2x) to `shot.png`, for comparison with
//! refs/home/dark@1280x832.png. Run: `cargo run -p ui --example shot`.

use gpui::{AnyView, AppContext as _, HeadlessAppContext, IntoElement, Render, Window, px, size};
use std::sync::Arc;
use theme::Theme;

struct Root(AnyView);

impl Render for Root {
    fn render(&mut self, _: &mut Window, _: &mut gpui::Context<Self>) -> impl IntoElement {
        self.0.clone()
    }
}

fn main() -> anyhow::Result<()> {
    let text_system = gpui_platform::current_platform(true).text_system();
    let mut cx = HeadlessAppContext::with_platform(text_system, Arc::new(theme::Assets), || {
        gpui_platform::current_headless_renderer()
    });
    cx.update(|cx| {
        cx.set_global(Theme::dark());
        theme::load_fonts(cx);
    });
    let window = cx.open_window(size(px(1280.), px(832.)), |window, cx| {
        let view = ui::home_shell(window, cx);
        cx.new(|_| Root(view))
    })?;
    cx.run_until_parked();
    cx.capture_screenshot(window.into())?.save("shot.png")?;
    println!("shot.png");
    Ok(())
}
