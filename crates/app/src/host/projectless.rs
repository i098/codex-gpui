//! Original `nl`/`Ct` directory allocation and `Vl` projectless thread state.

use super::LocalHost;
use anyhow::{Context as _, Result, bail, ensure};
use serde_json::{Value, json};
use std::{
    fs,
    io::ErrorKind,
    path::{Path, PathBuf},
};

pub struct Workspace {
    pub cwd: String,
    output: String,
    pub root: String,
}

impl Workspace {
    pub fn instructions(&self) -> String {
        let output = &self.output;
        let files = if output != &self.cwd {
            format!(
                "Use work/ for intermediate files, scratch analysis, scripts, drafts, and temporary assets. Use {output} only for user-facing deliverables that should appear as outputs.\nWhen referring to saved deliverables in the final response, link only files from {output}."
            )
        } else {
            format!(
                "When using local files for this projectless thread, write scratch files, drafts, generated assets, and other outputs under {output}."
            )
        };
        format!(
            "### Projectless Chat\nThis projectless thread starts in a generated directory under the user's Documents/Codex folder.\nThe generated directory name is only a filesystem identifier. Do not infer the user's language, locale, or preferences from its name or path, even if it resembles a language code such as 'ru'.\nPrefer answering inline in chat unless using local files would make the result more useful.\n{files}\nDo not write directly in the home directory unless the user explicitly asks."
        )
    }
}

fn real_directory(path: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    ensure!(
        metadata.is_dir() && !metadata.is_symlink(),
        "Projectless thread directory must be a real directory"
    );
    Ok(())
}

impl LocalHost {
    pub(super) fn projectless_root(&self) -> Result<PathBuf> {
        let bridge = self
            .bridge
            .as_ref()
            .context("The app server is not connected")?;
        let config: Value =
            pollster::block_on(bridge.request("config/read", json!({"includeLayers": false})))?;
        let configured = &config["config"]["desktop"]["projectlessWorkspaceRoot"];
        let root = if configured.is_null() {
            PathBuf::from(
                std::env::var_os("HOME").context("No home directory for projectless tasks")?,
            )
            .join("Documents/Codex")
        } else {
            PathBuf::from(
                configured
                    .as_str()
                    .context("Projectless chat folder must be a path")?,
            )
        };
        ensure!(
            root.is_absolute(),
            "Projectless chat folder must be absolute on the execution host"
        );
        Ok(root)
    }

    pub(super) fn create_projectless_workspace(&self, prompt: &str) -> Result<Workspace> {
        let root = self.projectless_root()?;
        fs::create_dir_all(&root).context("create projectless workspace root")?;
        real_directory(&root)?;
        let date = root.join(chrono::Local::now().format("%Y-%m-%d").to_string());
        fs::create_dir_all(&date).context("create projectless date directory")?;
        real_directory(&date)?;
        let mut slug = prompt
            .to_lowercase()
            .split(|c: char| !c.is_ascii_alphanumeric())
            .filter(|word| !word.is_empty())
            .take(6)
            .collect::<Vec<_>>()
            .join("-");
        slug.truncate(80);
        if slug.is_empty() {
            slug = "new-chat".into();
        }
        for attempt in 0..105 {
            let name = match attempt {
                0 => slug.clone(),
                1..100 => format!("{slug}-{}", attempt + 1),
                _ => format!("{slug}-{}", crate::uuid_v4()),
            };
            let cwd = date.join(name);
            match fs::create_dir(&cwd) {
                Ok(()) => {}
                Err(error) if error.kind() == ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error).context("create projectless thread directory"),
            }
            let output = cwd.join("outputs");
            // The original keeps the new folder when split-directory creation fails.
            let outputs_created = fs::create_dir(&output);
            let work_created = fs::create_dir(cwd.join("work"));
            let output = if outputs_created.is_ok() && work_created.is_ok() {
                output
            } else {
                cwd.clone()
            };
            return Ok(Workspace {
                cwd: cwd.to_string_lossy().into_owned(),
                output: output.to_string_lossy().into_owned(),
                root: root.to_string_lossy().into_owned(),
            });
        }
        bail!("Unable to create a unique projectless thread directory")
    }

    pub(super) fn record_projectless_thread(
        &self,
        thread: &str,
        workspace: &Workspace,
    ) -> Result<()> {
        self.update_global_state(|state| {
            if state["projectless-thread-ids"].is_null() {
                state["projectless-thread-ids"] = json!([]);
            }
            let ids = state["projectless-thread-ids"]
                .as_array_mut()
                .context("projectless-thread-ids must be an array")?;
            if !ids.iter().any(|id| id == thread) {
                ids.push(json!(thread));
            }
            for (key, value) in [
                ("thread-workspace-root-hints", &workspace.root),
                ("thread-projectless-output-directories", &workspace.output),
            ] {
                if state[key].is_null() {
                    state[key] = json!({});
                }
                state[key]
                    .as_object_mut()
                    .with_context(|| format!("{key} must be an object"))?
                    .insert(thread.to_owned(), json!(value));
            }
            Ok(true)
        })
    }
}
