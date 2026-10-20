use gpui::{
    AppContext as _, Context, Entity, Focusable, HeadlessAppContext, IntoElement, Keystroke,
    ParentElement, Render, Styled, Window, div, px, size,
};
use std::sync::Arc;
use theme::{ActiveTheme as _, Theme};
use ui::{
    confirmation::Confirmation,
    settings_kit::{Choice, MultiSelect},
};

struct Preview {
    select: Entity<MultiSelect>,
    dialog: Option<Entity<Confirmation>>,
}

impl Render for Preview {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.theme().clone();
        div()
            .size_full()
            .bg(t.surface)
            .font_family(ui::SANS)
            .child(
                div()
                    .absolute()
                    .left(px(600.))
                    .top(px(200.))
                    .child(self.select.clone()),
            )
            .child(
                div()
                    .absolute()
                    .left(px(400.))
                    .top(px(300.))
                    .w(px(500.))
                    .child(ui::settings_kit::external_link_row(
                        "external",
                        "Documentation",
                        Some("Read the documentation in your browser".into()),
                        "https://developers.openai.com/codex",
                        window,
                        cx,
                    )),
            )
            .children(self.dialog.clone())
    }
}

fn main() -> anyhow::Result<()> {
    let args: Vec<_> = std::env::args().collect();
    let light = args.get(1).is_some_and(|value| value == "light");
    let mode = args.get(2).map_or("closed", String::as_str);
    let output = args.get(3).map_or("settings-kit.png", String::as_str);
    let mut cx =
        HeadlessAppContext::with_platform(textsys::text_system(), Arc::new(theme::Assets), || {
            gpui_platform::current_headless_renderer()
        });
    cx.update(|cx| {
        cx.set_global(if light { Theme::light() } else { Theme::dark() });
        theme::load_fonts(cx);
    });
    let window = cx.open_window(size(px(1280.), px(832.)), |window, cx| {
        let select = cx.new(|cx| {
            MultiSelect::new(
                "Available reasoning efforts",
                [
                    ("none", "None", true, true),
                    ("minimal", "Minimal", true, true),
                    ("low", "Low", true, true),
                    ("medium", "Medium", true, true),
                    ("high", "High", true, true),
                    ("xhigh", "Extra high", true, false),
                ]
                .into_iter()
                .map(|(value, label, checked, disabled)| Choice {
                    value: value.into(),
                    label: label.into(),
                    checked,
                    disabled,
                })
                .collect(),
                cx,
            )
        });
        select.read(cx).focus_handle(cx).focus(window, cx);
        let dialog = (mode == "confirmation").then(|| {
            cx.new(|cx| {
                Confirmation::new(
                    "Discard changes?",
                    "You have unsaved changes. If you leave now, your changes will be lost.",
                    "Discard changes",
                    window,
                    cx,
                )
                .destructive()
                .cancel_label("Keep editing")
            })
        });
        cx.new(|_| Preview { select, dialog })
    })?;
    cx.run_until_parked();
    if mode == "open" {
        cx.update_window(window.into(), |_, window, cx| {
            window.dispatch_keystroke(Keystroke::parse("down").unwrap(), cx);
        })?;
        cx.run_until_parked();
    }
    cx.capture_screenshot(window.into())?.save(output)?;
    println!("{output}");
    Ok(())
}
