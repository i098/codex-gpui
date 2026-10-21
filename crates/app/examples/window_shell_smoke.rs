use anyhow::{Context as _, Result, ensure};
use gpui::{
    AppContext as _, Context, Entity, Focusable as _, HeadlessAppContext, IntoElement,
    ParentElement as _, Render, Styled as _, Window, div, px, size,
};
use std::{path::PathBuf, sync::Arc, time::Duration};
use ui::screens::recovery::{Feedback, FeedbackEvent, Recovery, State};

struct Scene {
    recovery: Entity<Recovery>,
    feedback: Option<Entity<Feedback>>,
}

impl Render for Scene {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .relative()
            .child(self.recovery.clone())
            .children(self.feedback.clone())
    }
}

fn main() -> Result<()> {
    std::panic::set_hook(Box::new(|info| {
        eprintln!("{info}");
        std::process::exit(101);
    }));
    ensure!(
        cfg!(target_os = "macos"),
        "recovery surfaces require the Mac text renderer"
    );
    let output =
        PathBuf::from(std::env::var_os("CGX_RECOVERY_PROOF").context("set CGX_RECOVERY_PROOF")?);
    std::fs::create_dir_all(&output)?;
    let mut cx = HeadlessAppContext::with_platform(
        textsys::text_system(),
        Arc::new(theme::Assets),
        gpui_platform::current_headless_renderer,
    );
    cx.allow_parking();
    cx.update(|cx| {
        theme::load_fonts(cx);
        cx.set_global(theme::Theme::dark());
        cx.set_reduce_motion(true);
        ui::composer::Composer::set_caret_transparent(cx, true);
        app::init(cx);
    });
    let window = cx.open_window(size(px(1280.), px(832.)), |_, cx| {
        cx.new(|cx| Scene {
            recovery: cx.new(|cx| Recovery::new(State::Loading, cx)),
            feedback: None,
        })
    })?;
    for (name, state) in [
        ("startup", State::Loading),
        ("route-error", State::RouteError(String::new())),
        ("tab-error", State::TabError),
        (
            "server-crash",
            State::Crashed {
                configuration: false,
            },
        ),
        (
            "server-config",
            State::Crashed {
                configuration: true,
            },
        ),
        (
            "connection-error",
            State::ConnectionError(
                "Something went wrong connecting to the Codex CLI. Try restarting".into(),
            ),
        ),
    ] {
        window.update(&mut cx, |scene, _, cx| {
            scene
                .recovery
                .update(cx, |view, cx| view.set_state(state, cx))
        })?;
        cx.advance_clock(Duration::from_millis(500));
        cx.update_window(window.into(), |_, window, cx| window.draw(cx).clear(cx))?;
        cx.run_until_parked();
        cx.capture_screenshot(window.into())?
            .save(output.join(format!("{name}.png")))?;
        println!("wrote {name}.png");
    }
    window.update(&mut cx, |scene, window, cx| {
        scene.recovery.update(cx, |view, cx| {
            view.set_state(
                State::Crashed {
                    configuration: false,
                },
                cx,
            )
        });
        let feedback = cx.new(|cx| Feedback::new(window, cx));
        cx.subscribe(&feedback, |scene, _, event, cx| {
            if matches!(event, FeedbackEvent::Close) {
                scene.feedback = None;
                cx.notify();
            }
        })
        .detach();
        window.focus(&feedback.focus_handle(cx), cx);
        scene.feedback = Some(feedback);
        cx.notify();
    })?;
    cx.run_until_parked();
    cx.capture_screenshot(window.into())?
        .save(output.join("feedback.png"))?;
    println!("wrote feedback.png");
    cx.update_window(window.into(), |_, window, cx| {
        window.dispatch_keystroke(gpui::Keystroke::parse("escape").unwrap(), cx);
    })?;
    cx.run_until_parked();
    ensure!(
        window.update(&mut cx, |scene, _, _| scene.feedback.is_none())?,
        "Escape did not close feedback"
    );
    println!("PASS: feedback Escape closes the focused multiline field");
    Ok(())
}
