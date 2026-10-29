//! The composer's context window donut (the original's `contextUsage` indicator), shown when
//! `Show context window usage` is on and the open chat has reported `thread/tokenUsage/updated`.

use super::*;

/// The open chat's latest context window use, from `thread/tokenUsage/updated`
/// (`tokenUsage.last.totalTokens` of `tokenUsage.modelContextWindow`), as the original's `GH`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ContextUsage {
    pub used: u64,
    pub window: u64,
}

impl ContextUsage {
    /// `None` without a positive window or a token count, as the original's `GH`.
    pub fn from_token_usage(usage: &serde_json::Value) -> Option<Self> {
        let window = usage["modelContextWindow"].as_u64().filter(|w| *w > 0)?;
        let used = usage["last"]["totalTokens"].as_u64()?;
        Some(Self {
            used: used.min(window),
            window,
        })
    }

    /// Rounded percent of the window in use, 0 to 100.
    pub fn percent(self) -> u64 {
        (self.used as f64 / self.window as f64 * 100.).round() as u64
    }

    /// The tooltip, as the original's three centred lines.
    pub fn tooltip(self) -> String {
        let usage = self.percent();
        let status = if usage >= 50 {
            format!("{usage}% full")
        } else {
            format!("{usage}% used ({}% left)", 100 - usage)
        };
        let k = |n: u64| (n as f64 / 1000.).round() as u64;
        format!(
            "Context window:\n{status}\n{}k / {}k tokens used",
            k(self.used),
            k(self.window)
        )
    }
}

impl Composer {
    /// Shows `usage` in the footer, or hides the donut with `None`.
    pub fn set_context_usage(&mut self, usage: Option<ContextUsage>, cx: &mut Context<Self>) {
        if self.context_usage != usage {
            self.context_usage = usage;
            cx.notify();
        }
    }

    pub fn context_usage(&self) -> Option<ContextUsage> {
        self.context_usage
    }

    /// The 14 px donut (`icon-xs`, `text-codex-description`) with its tooltip.
    pub(super) fn context_donut(&self, t: &Theme) -> Option<impl IntoElement + use<>> {
        let usage = self.context_usage?;
        let fraction = usage.used as f32 / usage.window as f32;
        let (track, fill) = (t.text_tertiary.opacity(0.3), t.text_tertiary);
        let donut = gpui::canvas(
            |_, _, _| {},
            move |bounds, (), window, _| {
                let center = bounds.center();
                let radius = bounds.size.width.min(bounds.size.height) / 2. - px(1.5);
                let ring = |sweep: f32| {
                    let mut path = gpui::PathBuilder::stroke(px(2.));
                    let steps = (sweep * 48.).ceil().max(1.) as usize;
                    for step in 0..=steps {
                        // Clockwise from 12 o'clock.
                        let angle = std::f32::consts::TAU * sweep * step as f32 / steps as f32
                            - std::f32::consts::FRAC_PI_2;
                        let p = point(
                            center.x + radius * angle.cos(),
                            center.y + radius * angle.sin(),
                        );
                        if step == 0 {
                            path.move_to(p);
                        } else {
                            path.line_to(p);
                        }
                    }
                    path.build().ok()
                };
                if let Some(path) = ring(1.) {
                    window.paint_path(path, track);
                }
                if fraction > 0.
                    && let Some(path) = ring(fraction.min(1.))
                {
                    window.paint_path(path, fill);
                }
            },
        )
        .size(px(14.));
        let trigger = div()
            .id("context-usage")
            .role(gpui::Role::Image)
            .aria_label(format!("Context usage: {}%", usage.percent()))
            .size(px(28.))
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .child(donut);
        Some(crate::tooltip::tooltip(
            trigger,
            usage.tooltip(),
            None,
            crate::tooltip::Side::Top,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::ContextUsage;

    #[test]
    fn usage_follows_the_last_turn_and_clamps_to_the_window() {
        let usage = |v| ContextUsage::from_token_usage(&v);
        // The recorded 0.164 payload: `last` is the context in use, not the thread total.
        let live = usage(serde_json::json!({
            "total": {"totalTokens": 60217}, "last": {"totalTokens": 30681},
            "modelContextWindow": 828400,
        }))
        .unwrap();
        assert_eq!(live.percent(), 4);
        assert_eq!(
            live.tooltip(),
            "Context window:\n4% used (96% left)\n31k / 828k tokens used"
        );
        let full = usage(serde_json::json!({
            "last": {"totalTokens": 900}, "modelContextWindow": 600,
        }))
        .unwrap();
        assert_eq!((full.used, full.percent()), (600, 100));
        assert!(full.tooltip().contains("\n100% full\n"));
        assert_eq!(
            usage(serde_json::json!({"last": {"totalTokens": 1}, "modelContextWindow": 0})),
            None
        );
        assert_eq!(
            usage(serde_json::json!({"last": {"totalTokens": 1}, "modelContextWindow": null})),
            None
        );
    }
}
