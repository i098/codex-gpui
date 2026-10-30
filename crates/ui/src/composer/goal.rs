//! `/goal`: the open chat's persistent goal (`thread/goal/*`) as a footer pill. Clicking the
//! pill clears the goal.

use super::*;

/// A chat's goal: its objective and `active` / `paused` / `budgetLimited` / `complete` status.
#[derive(Clone, Debug, PartialEq)]
pub struct Goal {
    pub objective: String,
    pub status: String,
}

impl Goal {
    /// `None` without an objective, as the server reports a cleared goal.
    pub fn from_value(goal: &Value) -> Option<Self> {
        let objective = goal["objective"].as_str().filter(|text| !text.is_empty())?;
        Some(Self {
            objective: objective.into(),
            status: goal["status"].as_str().unwrap_or("active").into(),
        })
    }

    pub fn achieved(&self) -> bool {
        self.status == "complete"
    }

    pub fn label(&self) -> String {
        match self.status.as_str() {
            "complete" => "Goal achieved".into(),
            "paused" => format!("Goal paused: {}", self.objective),
            "budgetLimited" => format!("Goal limit reached: {}", self.objective),
            _ => format!("Goal: {}", self.objective),
        }
    }
}

impl Composer {
    /// Shows `goal` in the footer, or hides the pill with `None`.
    pub fn set_goal(&mut self, goal: Option<Goal>, cx: &mut Context<Self>) {
        if self.slash.goal != goal {
            self.slash.goal = goal;
            cx.notify();
        }
    }

    pub(super) fn goal_pill(&self, t: &Theme, cx: &mut Context<Self>) -> Option<Stateful<Div>> {
        let goal = self.slash.goal.as_ref()?;
        let label = goal.label();
        Some(
            div()
                .id("goal-pill")
                .role(gpui::Role::Button)
                .aria_label(format!("{label}. Clear goal"))
                .flex()
                .flex_initial()
                .min_w_0()
                .max_w(px(220.))
                .h(px(24.))
                .px(px(8.))
                .mr(px(4.))
                .gap(px(4.))
                .items_center()
                .rounded(px(12.))
                .border_1()
                .border_color(t.text.opacity(0.082))
                .cursor_pointer()
                .child(icon("thread-summary", 14., t.text_tertiary).flex_none())
                .child(text(label, 12., 16., 400., t.text).truncate())
                .on_click(cx.listener(|_, _, _, cx| cx.emit(SlashCommand::GoalClear))),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::Goal;

    #[test]
    fn goal_label_follows_status() {
        let goal =
            |status| Goal::from_value(&serde_json::json!({"objective": "ship", "status": status}));
        assert_eq!(goal("active").unwrap().label(), "Goal: ship");
        assert_eq!(goal("paused").unwrap().label(), "Goal paused: ship");
        assert!(goal("complete").unwrap().achieved());
        assert!(Goal::from_value(&serde_json::json!({"objective": ""})).is_none());
    }
}
