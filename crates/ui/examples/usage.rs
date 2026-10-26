use gpui::{
    AnyView, AppContext as _, BorrowAppContext as _, HeadlessAppContext, IntoElement, Render,
    Window, px, size,
};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use theme::Theme;
use ui::screens::{
    computer_use::Load,
    settings::{
        self,
        usage::{Access, Action, Limits, State, analytics::Snapshot},
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
    let mode = std::env::args().nth(2);
    let account_switch = mode.as_deref() == Some("account-switch");
    let confirm_reset = mode.as_deref() == Some("confirm-reset") || account_switch;
    let analytics = mode.as_deref() == Some("analytics");
    let reset_error = mode.as_deref() == Some("reset-error");
    let mut limits: Limits = ui::fixture::response("readback.jsonl", "account/rateLimits/read", 0);
    let plan = limits.rate_limits.plan_type.clone().unwrap_or_default();
    if reset_error {
        limits.rate_limit_reset_credits = None;
    }
    anyhow::ensure!(
        matches!(plan.as_str(), "plus" | "pro" | "prolite" | "promax"),
        "The recording must contain a paid consumer plan"
    );
    let oracle_path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tools/route1/oracle.json");
    let oracle: serde_json::Value = serde_json::from_slice(&std::fs::read(oracle_path)?)?;
    let payload = oracle["shared statsig_evaluations"]["payload"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("The original gate recording is missing"))?;
    let gates: serde_json::Value = serde_json::from_str(payload)?;
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
                analytics: gates["feature_gates"]["4027330399"]["value"] == true,
                app_limits: gates["feature_gates"]["185496087"]["value"] == true,
                balance_history: gates["feature_gates"]["2125321167"]["value"] == true
                    && gates["feature_gates"]["393259752"]["value"] == true,
                maximum_credits: 250_000,
                ..Default::default()
            },
            plan,
            limits: Load::Ready(Arc::new(limits)),
            reload: Load::Failed("No automatic reload response is present in the recording".into()),
            app_limits: Load::Failed("The original fixture cannot load app limits".into()),
            initial_analytics: analytics,
            analytics: Snapshot {
                usage: Load::Failed("The original fixture cannot load usage data".into()),
                credits: Load::Failed("The original fixture cannot load credit history".into()),
                plugins: Load::Failed("The original fixture cannot load plugin activity".into()),
                skills: Load::Failed("The original fixture cannot load skill activity".into()),
                messages: Load::Failed("The original fixture cannot load messages".into()),
                plan: Load::Failed("The original fixture cannot load plan history".into()),
                chats: Load::Failed("The original fixture cannot load chat usage".into()),
                token_usage: Load::Ready(Arc::new(ui::fixture::response(
                    "readback.jsonl",
                    "account/usage/read",
                    0,
                ))),
            },
            reference_time: Some(1_792_083_600),
            dispatch: Some(Arc::new(move |action, cx| match action {
                Action::Reset(_) => {
                    sent.fetch_add(1, Ordering::SeqCst);
                }
                Action::OpenUrl(url) => cx.open_url(&url),
                _ => {}
            })),
            ..Default::default()
        });
    });
    let config: bridge::types::ConfigReadResponse =
        ui::fixture::response("readback.jsonl", "config/read", 0);
    let skills: serde_json::Value = ui::fixture::response("readback.jsonl", "skills/list", 0);
    let window = cx.open_window(size(px(1280.), px(832.)), |_, cx| {
        let (shell, _) = settings::shell(&config.config, &skills, Some("Usage & billing"), cx);
        shell.update(cx, |shell, _| shell.can_go_back = true);
        cx.new(|_| Preview(shell.into()))
    })?;
    cx.run_until_parked();
    let click = |cx: &mut HeadlessAppContext, x: f32, y: f32| -> anyhow::Result<()> {
        cx.update_window(window.into(), |_, window, cx| {
            let position = gpui::point(px(x), px(y));
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
    if mode.as_deref() == Some("links") {
        for (x, y, url) in [
            (1113., 326., "https://chatgpt.com/pricing/"),
            (
                1117.,
                658.,
                "https://chatgpt.com/codex/settings/usage?credit_modal=true",
            ),
            (850., 278., "https://chatgpt.com/#settings/Billing"),
        ] {
            click(&mut cx, x, y)?;
            anyhow::ensure!(
                cx.take_opened_url().as_deref() == Some(url),
                "The clicked link must open {url}"
            );
        }
    }
    if confirm_reset || reset_error {
        cx.update_window(window.into(), |_, window, cx| {
            window.dispatch_event(
                gpui::PlatformInput::ScrollWheel(gpui::ScrollWheelEvent {
                    position: gpui::point(px(1100.), px(600.)),
                    delta: gpui::ScrollDelta::Pixels(gpui::point(px(0.), px(-10000.))),
                    touch_phase: gpui::TouchPhase::Moved,
                    ..Default::default()
                }),
                cx,
            );
        })?;
        cx.run_until_parked();
    }
    if confirm_reset {
        click(&mut cx, 1124., 776.)?;
        anyhow::ensure!(
            reset_requests.load(Ordering::SeqCst) == 0,
            "The first click must only confirm"
        );
        cx.capture_screenshot(window.into())?.save(&output)?;
        if account_switch {
            cx.update(|cx| {
                cx.update_global::<State, _>(|state, _| state.account_revision += 1);
            });
            cx.run_until_parked();
            click(&mut cx, 1124., 776.)?;
            anyhow::ensure!(
                reset_requests.load(Ordering::SeqCst) == 0,
                "An account change must discard the old reset confirmation"
            );
        }
        click(&mut cx, 1124., 776.)?;
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
