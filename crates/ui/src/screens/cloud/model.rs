use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashSet};

pub const UNIVERSAL: &str = "wham-public/wham-universal";
const REDACTED: &str = "<REDACTED>";

#[derive(Deserialize)]
pub struct EnvironmentName {
    pub id: String,
    pub label: String,
}

#[derive(Clone, Deserialize, Serialize)]
pub struct Environment {
    pub id: String,
    pub label: String,
    #[serde(default)]
    pub description: String,
    pub machine_id: String,
    #[serde(default)]
    pub machine: Value,
    pub repos: Vec<String>,
    #[serde(default)]
    pub repo_map: BTreeMap<String, Value>,
    pub github_connector_id: Option<String>,
    #[serde(default = "workspace")]
    pub workspace_dir: String,
    #[serde(default)]
    pub agent_network_access: Value,
    #[serde(default)]
    pub setup: Value,
    #[serde(default)]
    pub maintenance_setup: Value,
    #[serde(default)]
    pub env_vars: BTreeMap<String, String>,
    #[serde(default)]
    pub secrets: BTreeMap<String, Value>,
    pub secrets_with_domains: Option<Vec<SecretName>>,
    #[serde(default = "private")]
    pub share_settings: String,
    #[serde(default)]
    pub share_targets: Vec<Value>,
    #[serde(default)]
    pub auto_setup_settings: Value,
    #[serde(default)]
    pub cache_settings: Value,
    #[serde(default)]
    pub enable_authtranslator: bool,
    #[serde(default)]
    pub permissions: Value,
    #[serde(default)]
    pub creator: Value,
    pub created_at: Option<f64>,
    pub task_count: Option<u64>,
    #[serde(default)]
    pub etag: Value,
}

fn workspace() -> String {
    "/workspace".into()
}
fn private() -> String {
    "private".into()
}

#[derive(Clone, Deserialize, Serialize)]
pub struct SecretName {
    pub name: String,
    pub domain: Option<String>,
}

#[derive(Clone, PartialEq)]
pub struct Secret {
    pub name: String,
    pub value: String,
    pub domain: Option<String>,
    pub previous: Option<SecretName>,
}

impl PartialEq for SecretName {
    fn eq(&self, other: &Self) -> bool {
        self.name == other.name && self.domain == other.domain
    }
}

#[derive(Clone, PartialEq)]
pub struct Draft {
    pub label: String,
    pub description: String,
    pub machine: String,
    pub repository: String,
    pub repository_name: String,
    pub connector: Option<String>,
    pub workspace: String,
    pub network: Value,
    pub setup: String,
    pub maintenance: String,
    pub variables: Vec<(String, String)>,
    pub secrets: Vec<Secret>,
    pub sharing: String,
    pub editors: Vec<String>,
    pub auto_setup: Value,
    pub cache: Value,
    pub authtranslator: bool,
}

pub struct Runtime {
    pub name: &'static str,
    pub variable: &'static str,
    pub default: &'static str,
    pub versions: &'static [&'static str],
}

pub const RUNTIMES: &[Runtime] = &[
    Runtime {
        name: "Python",
        variable: "CODEX_ENV_PYTHON_VERSION",
        default: "3.12",
        versions: &["3.14", "3.13", "3.12", "3.11", "3.10"],
    },
    Runtime {
        name: "Node.js",
        variable: "CODEX_ENV_NODE_VERSION",
        default: "20",
        versions: &["22", "20", "18"],
    },
    Runtime {
        name: "Ruby",
        variable: "CODEX_ENV_RUBY_VERSION",
        default: "3.4.4",
        versions: &["3.4.4", "3.3.8", "3.2.3"],
    },
    Runtime {
        name: "Rust",
        variable: "CODEX_ENV_RUST_VERSION",
        default: "1.89.0",
        versions: &[
            "1.95.0", "1.94.0", "1.93.0", "1.92.0", "1.91.1", "1.90.0", "1.89.0", "1.88.0",
            "1.87.0", "1.86.0", "1.85.1", "1.84.1", "1.83.0",
        ],
    },
    Runtime {
        name: "Go",
        variable: "CODEX_ENV_GO_VERSION",
        default: "1.24.3",
        versions: &["1.25.1", "1.24.3", "1.23.8", "1.22.12"],
    },
    Runtime {
        name: "Bun",
        variable: "CODEX_ENV_BUN_VERSION",
        default: "1.2.14",
        versions: &["1.2.14"],
    },
    Runtime {
        name: "PHP",
        variable: "CODEX_ENV_PHP_VERSION",
        default: "8.4",
        versions: &["8.5", "8.4", "8.3", "8.2"],
    },
    Runtime {
        name: "Java",
        variable: "CODEX_ENV_JAVA_VERSION",
        default: "21",
        versions: &["25", "24", "23", "22", "21", "17", "11"],
    },
    Runtime {
        name: "Swift",
        variable: "CODEX_ENV_SWIFT_VERSION",
        default: "6.1",
        versions: &["6.2", "6.1", "5.10"],
    },
];

impl Default for Draft {
    fn default() -> Self {
        Self {
            label: String::new(),
            description: String::new(),
            machine: UNIVERSAL.into(),
            repository: String::new(),
            repository_name: String::new(),
            connector: None,
            workspace: workspace(),
            network: json!({"mode":"off"}),
            setup: String::new(),
            maintenance: String::new(),
            variables: RUNTIMES
                .iter()
                .map(|r| (r.variable.into(), r.default.into()))
                .collect(),
            secrets: Vec::new(),
            sharing: private(),
            editors: Vec::new(),
            auto_setup: json!({"use_auto_setup":true}),
            cache: json!({"post_setup_cache_enabled":true}),
            authtranslator: false,
        }
    }
}

impl Environment {
    pub fn repository_name(&self) -> String {
        self.repos
            .iter()
            .map(|id| {
                self.repo_map
                    .get(id)
                    .and_then(|v| v["repository_full_name"].as_str())
                    .unwrap_or(id)
            })
            .collect::<Vec<_>>()
            .join(", ")
    }

    pub fn secret_names(&self) -> Vec<SecretName> {
        self.secrets_with_domains.clone().unwrap_or_else(|| {
            self.secrets
                .keys()
                .map(|key| match key.split_once("::") {
                    Some((domain, name)) => SecretName {
                        name: if name.is_empty() {
                            key.clone()
                        } else {
                            name.into()
                        },
                        domain: normalized_domain(Some(domain)),
                    },
                    None => SecretName {
                        name: key.clone(),
                        domain: None,
                    },
                })
                .collect()
        })
    }
}

impl From<&Environment> for Draft {
    fn from(e: &Environment) -> Self {
        let mut network = e.agent_network_access.clone();
        if network.is_null() {
            network = json!({"mode":"off"});
        }
        if network["mode"] == "on" && network["preset_allowlist"].is_null() {
            network["preset_allowlist"] = "all".into();
        }
        let setup = script(&e.setup);
        Self {
            label: e.label.clone(),
            description: e.description.clone(),
            machine: e.machine_id.clone(),
            repository: e.repos.first().cloned().unwrap_or_default(),
            repository_name: e.repository_name(),
            connector: e.github_connector_id.clone(),
            workspace: e.workspace_dir.clone(),
            network,
            setup: setup.clone(),
            maintenance: script(&e.maintenance_setup),
            variables: e.env_vars.clone().into_iter().collect(),
            secrets: e
                .secret_names()
                .into_iter()
                .map(|name| Secret {
                    name: name.name.clone(),
                    domain: name.domain.clone(),
                    value: REDACTED.into(),
                    previous: Some(name),
                })
                .collect(),
            sharing: if e.share_settings == "public" {
                "workspace".into()
            } else {
                e.share_settings.clone()
            },
            editors: e
                .share_targets
                .iter()
                .filter_map(|v| v["email"].as_str().map(str::to_owned))
                .collect(),
            auto_setup: if e.auto_setup_settings.is_null() {
                json!({"use_auto_setup":setup.is_empty()})
            } else {
                e.auto_setup_settings.clone()
            },
            cache: if e.cache_settings.is_null() {
                json!({"post_setup_cache_enabled":true})
            } else {
                e.cache_settings.clone()
            },
            authtranslator: e.enable_authtranslator,
        }
    }
}

fn script(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        Value::Array(parts) => parts
            .iter()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

fn normalized_domain(domain: Option<&str>) -> Option<String> {
    domain
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

impl Draft {
    pub fn errors(
        &self,
        environments: &[EnvironmentName],
        current: Option<&str>,
    ) -> BTreeMap<&'static str, &'static str> {
        let mut errors = BTreeMap::new();
        let name = self.label.trim();
        if name.is_empty() {
            errors.insert("name", "Enter an environment name");
        } else if name.encode_utf16().count() > 64 {
            errors.insert("name", "Use 64 characters or fewer");
        } else if environments
            .iter()
            .any(|e| Some(e.id.as_str()) != current && e.label == name)
        {
            errors.insert("name", "An environment with this name already exists");
        }
        if self.description.encode_utf16().count() > 512 {
            errors.insert("description", "Use 512 characters or fewer");
        }
        if !self.workspace.trim().starts_with('/') || self.workspace.trim() == "/" {
            errors.insert(
                "workspace",
                "Enter an absolute directory such as /workspace, not the root directory",
            );
        }
        if self.repository.is_empty() {
            errors.insert("repository", "Select a repository");
        }
        if self.machine.is_empty() {
            errors.insert("machine", "Select a container image");
        }
        if self.network["mode"] != "off"
            && self.network["preset_allowlist"]
                .as_str()
                .is_none_or(|p| p == "none")
            && !self.network["allowlist_domains"].as_str().is_some_and(|s| {
                s.split(|c: char| c == ',' || c.is_whitespace())
                    .any(|s| !s.is_empty())
            })
        {
            errors.insert("network", "Enter at least one allowed domain");
        }
        let mut variables = HashSet::new();
        if self
            .variables
            .iter()
            .any(|(k, v)| k.trim().is_empty() || v.trim().is_empty() || !variables.insert(k.trim()))
        {
            errors.insert(
                "variables",
                "Variable names and values are required, and names must be unique",
            );
        }
        let mut secrets = HashSet::new();
        if self.secrets.iter().any(|s| {
            s.name.trim().is_empty()
                || s.value.trim().is_empty()
                || !secrets.insert((s.name.trim(), normalized_domain(s.domain.as_deref())))
                || (s.value == REDACTED
                    && s.previous.as_ref().is_some_and(|p| {
                        p.domain.is_none()
                            && normalized_domain(s.domain.as_deref()).is_none()
                            && p.name != s.name.trim()
                    }))
        }) {
            errors.insert(
                "secrets",
                "Secret names and values are required, and names must be unique within each domain",
            );
        }
        if self.sharing == "workspace"
            && self
                .editors
                .iter()
                .filter(|e| !e.trim().is_empty())
                .any(|e| {
                    e.trim().split_once('@').is_none_or(|(name, host)| {
                        name.is_empty() || !host.contains('.') || e.chars().any(char::is_whitespace)
                    })
                })
        {
            errors.insert("editors", "Enter a valid email address for each editor");
        }
        errors
    }

    pub fn set_runtime(&mut self, runtime: &Runtime, value: Option<&str>) {
        self.variables.retain(|(k, _)| k.trim() != runtime.variable);
        if let Some(value) = value {
            self.variables.push((runtime.variable.into(), value.into()));
        }
    }

    pub fn payload(&self, original: Option<&Environment>) -> Value {
        let mut seen = HashSet::new();
        let editors: Vec<_> = self
            .editors
            .iter()
            .map(|s| s.trim())
            .filter(|s| !s.is_empty() && seen.insert(s.to_lowercase()))
            .map(|email| json!({"email":email,"permission":"editor","type":"user"}))
            .collect();
        let mut payload = json!({
            "label":self.label.trim(), "description":self.description, "machine_id":self.machine,
            "workspace_dir":self.workspace.trim(), "agent_network_access":self.network,
            "setup":self.setup, "maintenance_setup":self.maintenance,
            "env_vars":self.variables.iter().map(|(k,v)| (k.trim().to_owned(), v.clone())).collect::<BTreeMap<_,_>>(),
            "share_settings":self.sharing, "share_targets":if self.sharing == "private" { Vec::new() } else { editors },
            "auto_setup_settings":self.auto_setup, "cache_settings":self.cache, "enable_authtranslator":self.authtranslator,
        });
        if let Some(original) = original {
            payload["etag"] = original.etag.clone();
            payload["secrets_with_domains"] = self.secret_changes(original).into();
        } else {
            payload["repos"] = json!([self.repository]);
            payload["github_connector_id"] = json!(self.connector);
            payload["secrets_with_domains"] = self.secrets.iter().map(|s| json!({"name":s.name.trim(),"domain":normalized_domain(s.domain.as_deref()),"value":s.value})).collect::<Vec<_>>().into();
        }
        payload
    }

    fn secret_changes(&self, original: &Environment) -> Vec<Value> {
        let mut retained = HashSet::new();
        let mut changes = Vec::new();
        for secret in &self.secrets {
            let name = secret.name.trim();
            let domain = normalized_domain(secret.domain.as_deref());
            match &secret.previous {
                None if !secret.value.is_empty() => {
                    retained.insert((name.to_owned(), domain.clone()));
                    changes.push(json!({"name":name,"domain":domain,"value":secret.value}));
                }
                Some(previous) => {
                    let old_domain = normalized_domain(previous.domain.as_deref());
                    retained.insert((previous.name.clone(), old_domain.clone()));
                    if previous.name != name || old_domain != domain || secret.value != REDACTED {
                        let mut change = json!({"name":name,"domain":domain,"value":if secret.value == REDACTED { None } else { Some(&secret.value) },"previous_domain":old_domain});
                        if previous.name != name {
                            change["previous_name"] = previous.name.clone().into();
                        }
                        changes.push(change);
                    }
                }
                _ => {}
            }
        }
        for secret in original.secret_names() {
            if !retained.contains(&(
                secret.name.clone(),
                normalized_domain(secret.domain.as_deref()),
            )) {
                changes.push(json!({"name":secret.name,"domain":null,"value":null,"previous_domain":secret.domain}));
            }
        }
        changes
    }
}

pub fn parse_assignments(text: &str) -> Option<Vec<(String, String)>> {
    let mut entries = Vec::new();
    for line in text.lines().map(str::trim) {
        if line.is_empty() || line.starts_with(['#', ';']) {
            continue;
        }
        let line = line
            .strip_prefix("export ")
            .map(str::trim_start)
            .unwrap_or(line);
        let (key, raw) = line.split_once('=')?;
        let key = key.trim();
        if key.is_empty()
            || !key
                .chars()
                .all(|c| c.is_alphanumeric() || matches!(c, '_' | '.' | '-'))
        {
            return None;
        }
        let raw = raw.trim();
        let value = if raw.starts_with(['\'', '"']) {
            let quote = raw.chars().next()?;
            let end = raw[1..].rfind(quote)? + 1;
            let rest = raw[end + 1..].trim_start();
            if !rest.is_empty() && !rest.starts_with('#') {
                return None;
            }
            raw[1..end].to_owned()
        } else {
            let comment = raw
                .char_indices()
                .find(|(index, c)| *c == '#' && (*index == 0 || raw.as_bytes()[index - 1] != b'\\'))
                .map(|(index, _)| index)
                .unwrap_or(raw.len());
            raw[..comment].trim_end().to_owned()
        };
        entries.push((key.to_owned(), value));
    }
    (!entries.is_empty()).then_some(entries)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn environment() -> Environment {
        serde_json::from_value(json!({
            "id":"env-1","label":"Example","machine_id":UNIVERSAL,"repos":["github-repository"],
            "github_connector_id":null,"secrets_with_domains":[
                {"name":"TOKEN","domain":"example.com"},{"name":"REMOVE","domain":null}
            ]
        }))
        .unwrap()
    }

    #[test]
    fn saving_preserves_unchanged_secret_values_and_explicitly_deletes_removed_secrets() {
        let original = environment();
        let mut draft = Draft::from(&original);
        draft.secrets.remove(1);
        let body = draft.payload(Some(&original));
        assert_eq!(
            body["secrets_with_domains"],
            json!([{"name":"REMOVE","domain":null,"value":null,"previous_domain":null}])
        );
        assert!(!body.to_string().contains(REDACTED));
    }

    #[test]
    fn renaming_a_global_secret_requires_a_new_value() {
        let original = environment();
        let mut draft = Draft::from(&original);
        draft.secrets[1].name = "RENAMED".into();
        assert!(draft.errors(&[], Some("env-1")).contains_key("secrets"));
        draft.secrets[1].value = "new-value".into();
        assert!(!draft.errors(&[], Some("env-1")).contains_key("secrets"));
        assert_eq!(
            draft.payload(Some(&original))["secrets_with_domains"],
            json!([{
                "name":"RENAMED","domain":null,"value":"new-value","previous_domain":null,"previous_name":"REMOVE"
            }])
        );
    }

    #[test]
    fn env_paste_preserves_quotes_and_rejects_partial_parse() {
        assert_eq!(
            parse_assignments("# comment\nexport FIRST='a#b'\nSECOND=c # note\n"),
            Some(vec![
                ("FIRST".into(), "a#b".into()),
                ("SECOND".into(), "c".into())
            ])
        );
        assert_eq!(parse_assignments("FIRST=value\nnot an assignment"), None);
    }
}
