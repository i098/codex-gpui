use super::*;
use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ContextUsage {
    used: u64,
    capacity: u64,
}

impl ContextUsage {
    pub fn from_notification(value: &Value) -> Option<Self> {
        let capacity = value["modelContextWindow"].as_u64()?;
        let used = value["last"]["totalTokens"].as_u64()?;
        (capacity > 0).then_some(Self {
            used: used.min(capacity),
            capacity,
        })
    }

    fn percent(self) -> f64 {
        self.used as f64 / self.capacity as f64 * 100.
    }
}

pub(super) struct Indicator {
    usage: ContextUsage,
    scale: f32,
    color: Hsla,
    image: Arc<gpui::Image>,
    tooltip: SharedString,
}

impl Indicator {
    fn new(usage: ContextUsage, scale: f32, color: Hsla) -> Self {
        let percent = usage.percent();
        let rounded = percent.round() as u64;
        let ink: gpui::Rgba = color.into();
        let color_css = format!(
            "#{:02x}{:02x}{:02x}{:02x}",
            (ink.r * 255.).round() as u8,
            (ink.g * 255.).round() as u8,
            (ink.b * 255.).round() as u8,
            (ink.a * 255.).round() as u8
        );
        let size = (12. * scale).round().max(1.) as u32;
        let circumference = std::f64::consts::TAU * 5.;
        let offset = circumference * (1. - percent / 100.);
        let svg = format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="{size}" height="{size}" viewBox="0 0 12 12" color="{color_css}"><circle cx="6" cy="6" r="5" stroke="currentColor" stroke-width="2" fill="none" opacity=".16"/><circle cx="6" cy="6" r="5" stroke="currentColor" stroke-width="2" fill="none" opacity="{}" stroke-linecap="round" stroke-dasharray="{circumference}" stroke-dashoffset="{offset}" transform="rotate(-90 6 6)"/></svg>"#,
            u8::from(usage.used > 0)
        );
        let status = if rounded >= 50 {
            format!("{rounded}% full")
        } else {
            format!("{rounded}% used ({}% left)", 100 - rounded)
        };
        let tooltip = format!(
            "Context window:\n{status}\n{}k / {}k tokens used",
            (usage.used as f64 / 1000.).round() as u64,
            (usage.capacity as f64 / 1000.).round() as u64
        )
        .into();
        Self {
            usage,
            scale,
            color,
            image: Arc::new(gpui::Image::from_bytes(
                gpui::ImageFormat::Svg,
                svg.into_bytes(),
            )),
            tooltip,
        }
    }
}

impl Composer {
    pub fn set_context_usage(&mut self, usage: Option<ContextUsage>, cx: &mut Context<Self>) {
        if self.context_usage != usage {
            self.context_usage = usage;
            *self.context_indicator.borrow_mut() = None;
            cx.notify();
        }
    }

    pub(super) fn context_control(
        &self,
        window: &Window,
        t: &Theme,
        cx: &App,
    ) -> Option<AnyElement> {
        if !cx
            .try_global::<Preferences>()
            .is_some_and(|preferences| preferences.show_context_usage)
        {
            return None;
        }
        let usage = self.context_usage?;
        let scale = window.scale_factor();
        let mut cache = self.context_indicator.borrow_mut();
        if cache.as_ref().is_none_or(|indicator| {
            indicator.usage != usage
                || indicator.scale != scale
                || indicator.color != t.text_tertiary
        }) {
            *cache = Some(Indicator::new(usage, scale, t.text_tertiary));
        }
        let indicator = cache.as_ref()?;
        let trigger = div()
            .id("context-usage")
            .size(px(12.))
            .rounded_full()
            .role(gpui::Role::Image)
            .aria_label(format!(
                "Context usage: {}%",
                usage.percent().round() as u64
            ))
            .track_focus(&self.context_focus)
            .focus_visible(crate::focus::trigger_focus)
            .child(gpui::img(indicator.image.clone()).size_full());
        Some(
            crate::tooltip::tooltip(
                trigger,
                indicator.tooltip.clone(),
                None,
                crate::tooltip::Side::Top,
            )
            .side_offset(px(4.))
            .focus_handle(&self.context_focus)
            .into_any_element(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usage_clamps_to_capacity_and_hides_unknown_or_invalid_windows() {
        assert_eq!(
            ContextUsage::from_notification(
                &json!({"modelContextWindow": 100, "last": {"totalTokens": 150}})
            ),
            Some(ContextUsage {
                used: 100,
                capacity: 100
            })
        );
        assert_eq!(
            ContextUsage::from_notification(
                &json!({"modelContextWindow": 0, "last": {"totalTokens": 2}})
            ),
            None
        );
        assert_eq!(
            ContextUsage::from_notification(
                &json!({"modelContextWindow": 100, "last": {"totalTokens": -1}})
            ),
            None
        );
        assert_eq!(
            ContextUsage::from_notification(&json!({"last": {"totalTokens": 2}})),
            None
        );
    }
}
