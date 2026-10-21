use gpui::{AnyView, AppContext as _, HeadlessAppContext, IntoElement, Render, Window, px, size};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use theme::Theme;
use ui::screens::{
    computer_use::Load,
    settings::{
        self,
        usage::{Access, Action, Limits, State},
    },
};

struct Preview(AnyView);
impl Render for Preview {
    fn render(&mut self, _: &mut Window, _: &mut gpui::Context<Self>) -> impl IntoElement {
        self.0.clone()
    }
}

fn main() -> anyhow::Result<()> {
    std::panic::set_hook(Box::new(|info| {
        eprintln!("{info}");
        std::process::exit(101);
    }));
    let output = std::env::args()
        .nth(1)
        .ok_or_else(|| anyhow::anyhow!("Pass the output PNG path"))?;
    let confirm_reset = std::env::args().nth(2).as_deref() == Some("confirm-reset");
    let limits: Limits = ui::fixture::response("readback.jsonl", "account/rateLimits/read", 0);
    let plan = limits.rate_limits.plan_type.clone().unwrap_or_default();
    anyhow::ensure!(
        matches!(plan.as_str(), "plus" | "pro" | "prolite" | "promax"),
        "The recording must contain a paid consumer plan"
    );
    let mut cx = HeadlessAppContext::with_platform(
        textsys::text_system(),
        Arc::new(theme::Assets),
        gpui_platform::current_headless_renderer,
    );
    let reset_requests = Arc::new(AtomicUsize::new(0));
    let sent = reset_requests.clone();
    cx.update(|cx| {
        cx.set_global(Theme::dark());
        theme::load_fonts(cx);
        cx.set_global(State {
            access: Access {
                visible: true,
                manage_credits: true,
                view_credits: true,
                maximum_credits: 250_000,
                ..Default::default()
            },
            plan,
            limits: Load::Ready(Arc::new(limits)),
            reload: Load::Failed("No automatic reload response is present in the recording".into()),
            dispatch: Some(Arc::new(move |action, _| {
                if matches!(action, Action::Reset(_)) {
                    sent.fetch_add(1, Ordering::SeqCst);
                }
            })),
            ..Default::default()
        });
    });
    let config: bridge::types::ConfigReadResponse =
        ui::fixture::response("readback.jsonl", "config/read", 0);
    let skills: serde_json::Value = ui::fixture::response("readback.jsonl", "skills/list", 0);
    let window = cx.open_window(size(px(1280.), px(832.)), |_, cx| {
        let (shell, _) = settings::shell(&config.config, &skills, Some("Usage & billing"), cx);
        cx.new(|_| Preview(shell.into()))
    })?;
    cx.run_until_parked();
    let click_reset = |cx: &mut HeadlessAppContext| -> anyhow::Result<()> {
        cx.update_window(window.into(), |_, window, cx| {
            let position = gpui::point(px(1124.), px(798.));
            window.dispatch_event(
                gpui::PlatformInput::MouseDown(gpui::MouseDownEvent {
                    button: gpui::MouseButton::Left,
                    position,
                    modifiers: Default::default(),
                    click_count: 1,
                    first_mouse: false,
                }),
                cx,
            );
            window.dispatch_event(
                gpui::PlatformInput::MouseUp(gpui::MouseUpEvent {
                    button: gpui::MouseButton::Left,
                    position,
                    modifiers: Default::default(),
                    click_count: 1,
                }),
                cx,
            );
        })?;
        cx.run_until_parked();
        Ok(())
    };
    if confirm_reset {
        click_reset(&mut cx)?;
        anyhow::ensure!(
            reset_requests.load(Ordering::SeqCst) == 0,
            "The first click must only confirm"
        );
        cx.capture_screenshot(window.into())?.save(&output)?;
        click_reset(&mut cx)?;
        anyhow::ensure!(
            reset_requests.load(Ordering::SeqCst) == 1,
            "The second click must request exactly one reset"
        );
    } else {
        cx.capture_screenshot(window.into())?.save(&output)?;
    }
    println!("{output}");
    Ok(())
}
