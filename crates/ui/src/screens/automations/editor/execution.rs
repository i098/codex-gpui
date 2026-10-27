use bridge::types::{Model, Thread};
use gpui::SharedString;
use serde_json::{Value, json};

use crate::screens::sidebar::{Project, title};

struct ModelChoice {
    id: String,
    label: SharedString,
    efforts: Vec<SharedString>,
}

pub(super) struct Execution {
    pub values: Value,
    changed: Vec<&'static str>,
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
    ) {
        self.models = models
            .iter()
            .filter(|model| !model.hidden)
            .map(|model| ModelChoice {
                id: model.id.clone(),
                label: model.display_name.clone().into(),
                efforts: model
                    .supported_reasoning_efforts
                    .iter()
                    .filter_map(|effort| {
                        effort["reasoningEffort"]
                            .as_str()
                            .map(|value| value.to_owned().into())
                    })
                    .collect(),
            })
            .collect();
        self.default_model = config["model"].as_str().map(str::to_owned).or_else(|| {
            models
                .iter()
                .find(|model| model.is_default)
                .map(|model| model.id.clone())
        });
        if let Some(projects) = projects {
            self.projects = projects
                .iter()
                .filter(|project| !project.missing && project.root.is_some())
                .cloned()
                .collect();
        }
        self.threads = threads
            .iter()
            .map(|thread| (thread.id.clone(), title(thread)))
            .collect();
        self.cwd = cwd.to_owned();
        if self.values["cwds"].as_array().is_none_or(Vec::is_empty) {
            self.values["cwds"] = json!([cwd]);
        }
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

    fn selected_model(&self) -> Option<&ModelChoice> {
        if let Some(id) = self.values["model"].as_str() {
            return self.models.iter().find(|model| model.id == id);
        }
        self.models
            .iter()
            .find(|model| Some(model.id.as_str()) == self.default_model.as_deref())
            .or_else(|| self.models.first())
    }

    pub fn model_count(&self) -> usize {
        self.models.len() + 1
    }
    pub fn model_label(&self, index: usize) -> SharedString {
        if index == 0 {
            "Default".into()
        } else {
            self.models[index - 1].label.clone()
        }
    }
    pub fn model_value(&self) -> SharedString {
        self.values["model"].as_str().map_or_else(
            || "Default".into(),
            |id| {
                self.models
                    .iter()
                    .find(|model| model.id == id)
                    .map_or_else(|| id.to_owned().into(), |model| model.label.clone())
            },
        )
    }
    pub fn select_model(&mut self, index: usize) {
        self.set(
            "model",
            if index == 0 {
                Value::Null
            } else {
                json!(self.models[index - 1].id)
            },
        );
        self.set("reasoningEffort", Value::Null);
    }

    pub fn effort_count(&self) -> usize {
        self.selected_model()
            .map_or(1, |model| model.efforts.len() + 1)
    }
    pub fn effort_label(&self, index: usize) -> SharedString {
        if index == 0 {
            "Default".into()
        } else {
            self.selected_model().unwrap().efforts[index - 1].clone()
        }
    }
    pub fn effort_value(&self) -> SharedString {
        self.values["reasoningEffort"]
            .as_str()
            .map_or_else(|| "Default".into(), |value| value.to_owned().into())
    }
    pub fn select_effort(&mut self, index: usize) {
        self.set(
            "reasoningEffort",
            if index == 0 {
                Value::Null
            } else {
                json!(self.effort_label(index).as_ref())
            },
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
