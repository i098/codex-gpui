use bridge::types::{Model, Thread};
use gpui::SharedString;
use serde_json::{Value, json};

use crate::screens::sidebar::{Project, title};

const LEGACY_MODEL: &str = "gpt-5.3-codex";
const AUTOMATION_MODEL: &str = "gpt-6.1-sol";
const DEFAULT_EFFORT: &str = "medium";

struct ModelChoice {
    id: String,
    source_label: String,
    label: SharedString,
    efforts: Vec<SharedString>,
    default_effort: String,
}

pub(super) struct Execution {
    pub values: Value,
    changed: Vec<&'static str>,
    inherited_model: bool,
    inherited_effort: bool,
    models: Vec<ModelChoice>,
    default_model: Option<String>,
    projects: Vec<Project>,
    threads: Vec<(String, SharedString)>,
    cwd: String,
}

impl Execution {
    pub fn new(item: Option<&Value>) -> Self {
        let empty = Value::Null;
        let item = item.unwrap_or(&empty);
        Self {
            values: json!({
                "kind": item["kind"].as_str().unwrap_or("cron"),
                "targetThreadId": item["targetThreadId"],
                "projectId": item["target"]["projectId"],
                "cwds": item["cwds"].as_array().cloned().unwrap_or_default(),
                "model": item["model"],
                "reasoningEffort": item["reasoningEffort"],
                "notificationPolicy": item["notificationPolicy"],
                "autoArchive": item["autoArchive"].as_bool().unwrap_or(false),
                "executionEnvironment": item["executionEnvironment"].as_str().unwrap_or("worktree"),
                "localEnvironmentConfigPath": item["localEnvironmentConfigPath"],
            }),
            changed: Vec::new(),
            inherited_model: item["model"].is_null(),
            inherited_effort: item["reasoningEffort"].is_null(),
            models: Vec::new(),
            default_model: None,
            projects: Vec::new(),
            threads: Vec::new(),
            cwd: String::new(),
        }
    }

    pub fn set_options(
        &mut self,
        models: &[Model],
        threads: &[Thread],
        projects: Option<&[Project]>,
        config: &Value,
        cwd: &str,
    ) -> (bool, bool, bool) {
        let models_changed = self.set_models(models, config);
        let projects_changed = projects.is_some_and(|projects| {
            self.projects.iter().ne(projects
                .iter()
                .filter(|project| !project.missing && project.root.is_some()))
        });
        let threads_changed = self.threads.len() != threads.len()
            || self
                .threads
                .iter()
                .zip(threads)
                .any(|((id, label), thread)| id != &thread.id || label != &title(thread));
        if projects_changed {
            self.projects = projects
                .unwrap()
                .iter()
                .filter(|project| !project.missing && project.root.is_some())
                .cloned()
                .collect();
        }
        if threads_changed {
            self.threads = threads
                .iter()
                .map(|thread| (thread.id.clone(), title(thread)))
                .collect();
        }
        if self.cwd != cwd {
            self.cwd = cwd.to_owned();
        }
        if self.values["cwds"].as_array().is_none_or(Vec::is_empty) {
            self.values["cwds"] = json!([cwd]);
        }
        self.resolve_model_settings();
        (models_changed, projects_changed, threads_changed)
    }

    fn set_models(&mut self, models: &[Model], config: &Value) -> bool {
        let selected = self.values["model"].as_str();
        let visible_model =
            |model: &&Model| !model.hidden || Some(model.model.as_str()) == selected;
        let visible = models.iter().filter(visible_model);
        let mut compared = visible.clone();
        let models_changed = self.models.iter().any(|saved| {
            compared.next().is_none_or(|model| {
                saved.id != model.model
                    || saved.source_label != model.display_name
                    || saved.default_effort != model.default_reasoning_effort
                    || saved.efforts.iter().map(|effort| effort.as_ref()).ne(model
                        .supported_reasoning_efforts
                        .iter()
                        .filter_map(|effort| effort["reasoningEffort"].as_str())
                        .filter(|effort| *effort != "persistent"))
            })
        }) || compared.next().is_some();
        let preferred = config["codex_automation_model_slug"]
            .as_str()
            .unwrap_or(AUTOMATION_MODEL);
        let default_model = visible
            .clone()
            .find(|model| model.model == preferred)
            .or_else(|| visible.clone().find(|model| model.is_default))
            .or_else(|| {
                visible
                    .clone()
                    .find(|model| model.model == AUTOMATION_MODEL)
            })
            .or_else(|| visible.clone().next())
            .map(|model| model.model.as_str());
        let default_changed = self.default_model.as_deref() != default_model;
        if models_changed {
            self.models = visible
                .map(|model| ModelChoice {
                    id: model.model.clone(),
                    source_label: model.display_name.clone(),
                    label: crate::composer::model_label(&model.display_name).into(),
                    default_effort: model.default_reasoning_effort.clone(),
                    efforts: model
                        .supported_reasoning_efforts
                        .iter()
                        .filter_map(|effort| {
                            effort["reasoningEffort"]
                                .as_str()
                                .filter(|value| *value != "persistent")
                                .map(|value| value.to_owned().into())
                        })
                        .collect(),
                })
                .collect();
        }
        if default_changed {
            self.default_model = default_model.map(str::to_owned);
        }
        models_changed || default_changed
    }

    pub fn new_chat(&self) -> bool {
        self.values["kind"] != "heartbeat"
    }

    pub fn valid(&self) -> bool {
        self.new_chat()
            || self.values["targetThreadId"]
                .as_str()
                .is_some_and(|id| !id.is_empty())
    }

    fn set(&mut self, field: &'static str, value: Value) {
        if self.values[field] != value {
            self.values[field] = value;
            if !self.changed.contains(&field) {
                self.changed.push(field);
            }
        }
    }

    pub fn toggle_new_chat(&mut self) {
        let kind = if self.new_chat() { "heartbeat" } else { "cron" };
        self.set("kind", json!(kind));
        self.set("targetThreadId", Value::Null);
        if kind == "cron" && !self.changed.contains(&"cwds") {
            self.changed.push("cwds");
        }
        self.resolve_model_settings();
    }

    pub fn toggle_archive(&mut self) {
        self.set("autoArchive", json!(self.values["autoArchive"] != true));
    }

    pub fn changes(&self) -> Value {
        Value::Object(
            self.changed
                .iter()
                .map(|field| ((*field).to_owned(), self.values[*field].clone()))
                .collect(),
        )
    }

    pub fn creation(&self) -> Value {
        let mut values = self.values.clone();
        if values["cwds"].as_array().is_none_or(Vec::is_empty) {
            values.as_object_mut().unwrap().remove("cwds");
        }
        values
    }

    fn resolve_model_settings(&mut self) {
        if !self.new_chat() {
            return;
        }
        if self.models.is_empty() {
            if self.inherited_model && self.values["model"] != LEGACY_MODEL {
                self.set("model", json!(LEGACY_MODEL));
            }
            if self.inherited_effort && self.values["reasoningEffort"] != DEFAULT_EFFORT {
                self.set("reasoningEffort", json!(DEFAULT_EFFORT));
            }
            return;
        }
        let (model_change, effort_change) = self.resolved_model_changes();
        if let Some(value) = model_change {
            self.set("model", value);
        }
        if let Some(value) = effort_change {
            self.set("reasoningEffort", value);
        }
    }

    fn resolved_model_changes(&self) -> (Option<Value>, Option<Value>) {
        let requested = if self.inherited_model {
            None
        } else {
            self.values["model"].as_str()
        };
        let explicit = requested.and_then(|id| self.models.iter().find(|model| model.id == id));
        let legacy = requested
            .is_none()
            .then(|| self.models.iter().find(|model| model.id == LEGACY_MODEL))
            .flatten();
        let model = explicit
            .or(legacy)
            .or_else(|| {
                self.models
                    .iter()
                    .find(|model| Some(model.id.as_str()) == self.default_model.as_deref())
            })
            .unwrap_or(&self.models[0]);
        let requested_effort = if self.inherited_effort {
            None
        } else {
            self.values["reasoningEffort"].as_str()
        };
        let wanted = if explicit.is_some() {
            requested_effort
        } else if legacy.is_some() {
            requested_effort.or(Some(DEFAULT_EFFORT))
        } else {
            None
        };
        let effort = wanted
            .filter(|value| model.efforts.iter().any(|effort| effort.as_ref() == *value))
            .or_else(|| {
                model
                    .efforts
                    .iter()
                    .find(|effort| effort.as_ref() == model.default_effort)
                    .map(|effort| effort.as_ref())
            })
            .or_else(|| model.efforts.first().map(|effort| effort.as_ref()));
        let model_change =
            (self.values["model"].as_str() != Some(model.id.as_str())).then(|| json!(model.id));
        let effort_change = (self.values["reasoningEffort"].as_str() != effort)
            .then(|| effort.map_or(Value::Null, |effort| json!(effort)));
        (model_change, effort_change)
    }

    fn selected_model(&self) -> Option<&ModelChoice> {
        self.values["model"]
            .as_str()
            .and_then(|id| self.models.iter().find(|model| model.id == id))
    }

    pub fn model_count(&self) -> usize {
        self.models.len()
    }
    pub fn model_label(&self, index: usize) -> SharedString {
        self.models[index].label.clone()
    }
    pub fn model_value(&self) -> SharedString {
        self.values["model"].as_str().map_or_else(
            || "Select model".into(),
            |id| {
                self.models.iter().find(|model| model.id == id).map_or_else(
                    || crate::composer::model_label(id).into(),
                    |model| model.label.clone(),
                )
            },
        )
    }
    pub fn select_model(&mut self, index: usize) {
        self.inherited_model = false;
        self.inherited_effort = false;
        self.set("model", json!(self.models[index].id));
        self.resolve_model_settings();
    }

    pub fn effort_count(&self) -> usize {
        self.selected_model().map_or(0, |model| model.efforts.len())
    }
    pub fn effort_label(&self, index: usize) -> SharedString {
        crate::composer::effort_label(self.selected_model().unwrap().efforts[index].as_ref())
            .to_owned()
            .into()
    }
    pub fn effort_value(&self) -> SharedString {
        self.values["reasoningEffort"].as_str().map_or_else(
            || "Unavailable".into(),
            |value| crate::composer::effort_label(value).to_owned().into(),
        )
    }
    pub fn select_effort(&mut self, index: usize) {
        self.inherited_model = false;
        self.inherited_effort = false;
        self.set(
            "reasoningEffort",
            json!(self.selected_model().unwrap().efforts[index].as_ref()),
        );
    }

    pub fn project_count(&self) -> usize {
        self.projects.len() + 1
    }
    pub fn project_label(&self, index: usize) -> SharedString {
        if index == 0 {
            "None".into()
        } else {
            self.projects[index - 1].label.clone()
        }
    }
    pub fn project_value(&self) -> SharedString {
        self.values["projectId"].as_str().map_or_else(
            || "None".into(),
            |id| {
                self.projects
                    .iter()
                    .find(|project| project.id.as_ref() == id)
                    .map_or_else(|| id.to_owned().into(), |project| project.label.clone())
            },
        )
    }
    pub fn select_project(&mut self, index: usize) {
        if index == 0 {
            self.set("projectId", Value::Null);
            if !self.cwd.is_empty() {
                self.set("cwds", json!([self.cwd]));
            }
        } else {
            let project = &self.projects[index - 1];
            let id = json!(project.id.as_ref());
            let roots = json!([project.root.as_ref().unwrap().as_ref()]);
            self.set("projectId", id);
            self.set("cwds", roots);
        }
    }

    pub fn thread_count(&self) -> usize {
        self.threads.len() + 1
    }
    pub fn thread_label(&self, index: usize) -> SharedString {
        if index == 0 {
            "Select chat".into()
        } else {
            self.threads[index - 1].1.clone()
        }
    }
    pub fn thread_value(&self) -> SharedString {
        self.values["targetThreadId"].as_str().map_or_else(
            || "Select chat".into(),
            |id| {
                self.threads
                    .iter()
                    .find(|thread| thread.0 == id)
                    .map_or_else(|| id.to_owned().into(), |thread| thread.1.clone())
            },
        )
    }
    pub fn select_thread(&mut self, index: usize) {
        if index > 0 {
            self.set("targetThreadId", json!(self.threads[index - 1].0));
        }
    }
    pub fn notification_value(&self) -> &'static str {
        self.notification_label(usize::from(
            self.values["notificationPolicy"] == "failed_runs_only",
        ))
    }

    pub fn notification_label(&self, index: usize) -> &'static str {
        if index == 1 {
            "Failed runs only"
        } else if self.new_chat() {
            "All runs"
        } else {
            "Important updates"
        }
    }
    pub fn select_notification(&mut self, index: usize) {
        self.set(
            "notificationPolicy",
            if index == 0 {
                Value::Null
            } else {
                json!("failed_runs_only")
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn switching_back_to_new_chat_supplies_a_workspace() {
        let mut execution = Execution::new(Some(&json!({
            "kind": "heartbeat", "targetThreadId": "existing-thread"
        })));
        execution.set_options(&[], &[], None, &Value::Null, "/tmp/project");
        execution.toggle_new_chat();
        let changes = execution.changes();
        assert_eq!(changes["kind"], "cron");
        assert_eq!(changes["cwds"], json!(["/tmp/project"]));
        assert!(changes["targetThreadId"].is_null());
    }

    #[test]
    fn unchanged_model_options_keep_the_picker_open() {
        let mut models = vec![Model {
            id: "catalog-entry".into(),
            model: "gpt-6-astra".into(),
            display_name: "Astra".into(),
            description: String::new(),
            hidden: false,
            default_reasoning_effort: "high".into(),
            supported_reasoning_efforts: vec![json!({"reasoningEffort":"high"})],
            is_default: true,
            input_modalities: None,
        }];
        let mut execution = Execution::new(None);
        assert_eq!(
            execution.set_options(&models, &[], None, &Value::Null, "/tmp/project"),
            (true, false, false)
        );
        execution.select_model(0);
        assert_eq!(execution.values["model"], "gpt-6-astra");
        assert_eq!(
            execution.set_options(&models, &[], None, &Value::Null, "/tmp/project"),
            (false, false, false)
        );
        models[0].display_name = "Updated Astra".into();
        assert_eq!(
            execution.set_options(&models, &[], None, &Value::Null, "/tmp/project"),
            (true, false, false)
        );
        assert_eq!(execution.model_value().as_ref(), "Updated Astra");
        execution.select_effort(0);
        assert_eq!(execution.changes()["reasoningEffort"], "high");
    }

    #[test]
    fn inherited_values_follow_the_original_catalog_precedence() {
        let model = |id: &str, default: bool, effort: &str| Model {
            id: id.into(),
            model: id.into(),
            display_name: id.into(),
            description: String::new(),
            hidden: false,
            default_reasoning_effort: effort.into(),
            supported_reasoning_efforts: ["low", "medium", "high"]
                .into_iter()
                .map(|value| json!({"reasoningEffort":value}))
                .collect(),
            is_default: default,
            input_modalities: None,
        };
        let models = [
            model("gpt-6.1-sol", true, "low"),
            model("gpt-5.6-terra", false, "medium"),
            model("gpt-5.3-codex", false, "low"),
        ];
        let config =
            json!({"codex_automation_model_slug":"gpt-5.6-terra","model":"unrelated-chat-model"});
        for (requested, effort, count, expected_model, expected_effort) in [
            (None, None, 3, "gpt-5.3-codex", "medium"),
            (None, None, 2, "gpt-5.6-terra", "medium"),
            (None, None, 1, "gpt-6.1-sol", "low"),
            (None, None, 0, "gpt-5.3-codex", "medium"),
            (
                Some("gpt-5.6-terra"),
                Some("high"),
                3,
                "gpt-5.6-terra",
                "high",
            ),
            (
                Some("gpt-5.6-terra"),
                Some("ultra"),
                3,
                "gpt-5.6-terra",
                "medium",
            ),
            (Some("retired"), Some("high"), 2, "gpt-5.6-terra", "medium"),
            (Some("gpt-5.3-codex"), None, 3, "gpt-5.3-codex", "low"),
        ] {
            let mut execution =
                Execution::new(Some(&json!({"model":requested,"reasoningEffort":effort})));
            execution.set_options(&models[..count], &[], None, &config, "/tmp/project");
            assert_eq!(execution.values["model"], expected_model);
            assert_eq!(execution.values["reasoningEffort"], expected_effort);
        }
        let mut arriving = Execution::new(None);
        arriving.set_options(&models[..2], &[], None, &Value::Null, "/tmp/project");
        assert_eq!(arriving.values["model"], "gpt-6.1-sol");
        arriving.set_options(&models[..2], &[], None, &config, "/tmp/project");
        assert_eq!(arriving.values["model"], "gpt-5.6-terra");
        assert_eq!(arriving.values["reasoningEffort"], "medium");
        arriving.select_model(0);
        arriving.set_options(&models[..2], &[], None, &config, "/tmp/project");
        assert_eq!(arriving.values["model"], "gpt-6.1-sol");
    }
}
