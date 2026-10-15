//! Headless render smoke: draws one GPUI view with real CoreText text through the Metal
//! headless renderer and writes `probe.png`. The parity harness grows from this.

use gpui::{
    AppContext, HeadlessAppContext, IntoElement, ParentElement, Render, Styled, div, px, rgb, size,
};
use std::sync::Arc;

struct Probe;

impl Render for Probe {
    fn render(&mut self, _: &mut gpui::Window, _: &mut gpui::Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .bg(rgb(0x111111))
            .text_color(rgb(0xfcfcfc))
            .text_size(px(28.))
            .child("What should we build?")
    }
}

fn main() -> anyhow::Result<()> {
    let text_system = gpui_platform::current_platform(true).text_system();
    let mut cx = HeadlessAppContext::with_platform(text_system, Arc::new(()), || {
        gpui_platform::current_headless_renderer()
    });
    let window = cx.open_window(size(px(1280.), px(832.)), |_, cx| cx.new(|_| Probe))?;
    cx.run_until_parked();
    let image = cx.capture_screenshot(window.into())?;
    image.save("probe.png")?;
    println!("probe.png {}x{}", image.width(), image.height());
    Ok(())
}
