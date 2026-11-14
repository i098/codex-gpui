use crate::{Root, Route, log_err, resume_if};
use anyhow::Result;
use gpui::{Context, Window};
use serde_json::{Value, json};
use ui::{composer::SlashCommand, toast::Toast};

impl Root {
    pub(super) fn goal_command(
        &mut self,
        command: &SlashCommand,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Route::Thread(id) = self.state.route.clone() else {
            return;
        };
        let (method, params) = match command {
            SlashCommand::ReadGoal | SlashCommand::RefreshGoal => {
                ("thread/goal/get", json!({"threadId": id}))
            }
            SlashCommand::ClearGoal => (
                "thread/goal/clear",
                json!({"threadId": id, "origin": "user"}),
            ),
            SlashCommand::Goal { objective, status } => {
                let mut params = json!({"threadId": id, "origin": "user", "status": status});
                if let Some(objective) = objective {
                    params["objective"] = objective.clone().into();
                }
                ("thread/goal/set", params)
            }
            _ => return,
        };
        let refresh = matches!(command, SlashCommand::RefreshGoal);
        let resume = !matches!(command, SlashCommand::ReadGoal | SlashCommand::RefreshGoal)
            && !self.loaded.contains(&id);
        let bridge = self.bridge.clone();
        let objective = match command {
            SlashCommand::Goal { objective, .. } => objective.clone(),
            _ => None,
        };
        let task = cx.spawn_in(window, async move |this, cx| -> Result<()> {
            let result = async {
                resume_if(resume, &bridge, &this, &id, cx).await?;
                bridge.request::<_, Value>(method, params).await
            }
            .await;
            this.update(cx, |this, cx| {
                if this.state.route != Route::Thread(id.clone()) {
                    return;
                }
                match &result {
                    Ok(value) => {
                        if let Some(objective) = &objective {
                            this.thread_composer
                                .update(cx, |composer, cx| composer.goal_saved(objective, cx));
                        }
                        let goal = value.get("goal").filter(|goal| !goal.is_null()).cloned();
                        if refresh {
                            this.update_goal(goal, cx);
                        } else {
                            this.show_goal(goal, cx);
                        }
                    }
                    Err(error) => {
                        this.toast(Toast::danger(format!("Goal request failed: {error}")), cx)
                    }
                }
            })?;
            result.map(drop)
        });
        log_err(task, cx);
    }

    pub(super) fn show_goal(&mut self, goal: Option<Value>, cx: &mut Context<Self>) {
        let message = goal.as_ref().map_or_else(
            || "No goal for this chat".to_owned(),
            |goal| {
                format!(
                    "Goal ({}): {}",
                    goal["status"].as_str().unwrap_or("unknown"),
                    goal["objective"].as_str().unwrap_or_default()
                )
            },
        );
        self.update_goal(goal, cx);
        self.toast(Toast::info(message), cx);
    }

    pub(super) fn update_goal(&mut self, goal: Option<Value>, cx: &mut Context<Self>) {
        let completed = goal
            .as_ref()
            .is_some_and(|goal| goal["status"] == "complete");
        let new_completion = goal.as_ref().is_some_and(|goal| {
            self.thread
                .as_ref()
                .is_some_and(|view| view.update(cx, |view, cx| view.record_goal(goal, cx)))
        });
        self.thread_composer.update(cx, |composer, cx| {
            composer.set_goal(if completed { None } else { goal }, cx)
        });
        if new_completion && let Route::Thread(id) = self.state.route.clone() {
            let bridge = self.bridge.clone();
            let task = cx.spawn(async move |_, _| {
                bridge
                    .request::<_, Value>("thread/goal/clear", json!({"threadId": id}))
                    .await
                    .map(drop)
            });
            log_err(task, cx);
        }
    }
}
