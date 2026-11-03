use anyhow::{Context as _, Result};
use gpui::Global;
use serde_json::{Map, Value};
use std::{
    fs,
    io::ErrorKind,
    path::PathBuf,
    sync::{Arc, Mutex, MutexGuard, PoisonError},
};

/// The desktop host's `settings.readAll` and `settings.write(key, value)` service.
/// Host settings never go to the app-server's `config/value/write` endpoint.
#[derive(Clone, Default)]
pub struct Settings {
    state: Arc<State>,
}

#[derive(Default)]
struct State {
    values: Mutex<Map<String, Value>>,
    path: Option<PathBuf>,
}

/// Applies the host preferences the app itself enforces.
///
/// `reduced-motion-preference`: `on` stills decorative motion; `system` and `off` leave it on,
/// as the host reads no OS preference. `dock-icon-preference` (macOS): `codex-system` and
/// `space-system` follow the system appearance, `app-default` restores the bundle icon.
/// Without a stored value the app keeps its state.
pub(crate) fn apply(cx: &mut gpui::App) {
    let Some(values) = cx
        .try_global::<Settings>()
        .map(|settings| settings.read_all().clone())
    else {
        return;
    };
    if let Some(motion) = values.get("reduced-motion-preference") {
        cx.set_reduce_motion(motion == "on");
    }
    let hidden = values.get("show-full-access-in-composer") == Some(&Value::Bool(false));
    cx.set_global(ui::composer::FullAccessHidden(hidden));
    if cfg!(target_os = "macos")
        && let Some(dock) = values.get("dock-icon-preference").and_then(Value::as_str)
    {
        let dark = matches!(
            cx.window_appearance(),
            gpui::WindowAppearance::Dark | gpui::WindowAppearance::VibrantDark
        );
        let name = match (dock, dark) {
            ("codex-system", true) => Some("dock/codex-dark.png"),
            ("codex-system", false) => Some("dock/codex-light.png"),
            ("space-system", true) => Some("dock/space-dark.png"),
            ("space-system", false) => Some("dock/space-light.png"),
            _ => None,
        };
        let png = name.and_then(theme::Assets::get);
        cx.set_dock_icon(png.as_ref().map(|file| file.data.as_ref()));
    }
}

impl Global for Settings {}

impl Settings {
    pub fn load(path: PathBuf) -> Result<Self> {
        let values = match fs::read(&path) {
            Ok(bytes) => serde_json::from_slice(&bytes).context("invalid host settings")?,
            Err(error) if error.kind() == ErrorKind::NotFound => Map::new(),
            Err(error) => return Err(error.into()),
        };
        Ok(Self {
            state: Arc::new(State {
                values: Mutex::new(values),
                path: Some(path),
            }),
        })
    }

    /// Drop this shared read guard before writing settings.
    pub fn read_all(&self) -> MutexGuard<'_, Map<String, Value>> {
        self.state
            .values
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    pub fn write(&mut self, key: &str, value: Value) -> Result<()> {
        self.update(key, |_| Ok(value))
    }

    /// Writes every pair under one lock and one save; a failed save restores all of them.
    pub fn write_batch<'a>(
        &mut self,
        pairs: impl IntoIterator<Item = (&'a str, Value)>,
    ) -> Result<()> {
        let mut values = self.read_all();
        let previous: Vec<_> = pairs
            .into_iter()
            .map(|(key, value)| (key.to_owned(), values.insert(key.to_owned(), value)))
            .collect();
        if let Err(error) = self.save(&values) {
            for (key, value) in previous.into_iter().rev() {
                match value {
                    Some(value) => {
                        values.insert(key, value);
                    }
                    None => {
                        values.remove(&key);
                    }
                }
            }
            return Err(error);
        }
        Ok(())
    }

    pub(crate) fn update(
        &mut self,
        key: &str,
        change: impl FnOnce(Option<&Value>) -> Result<Value>,
    ) -> Result<()> {
        let mut values = self.read_all();
        let value = change(values.get(key))?;
        let old = values.insert(key.to_owned(), value);
        if let Err(error) = self.save(&values) {
            match old {
                Some(value) => {
                    values.insert(key.to_owned(), value);
                }
                None => {
                    values.remove(key);
                }
            }
            return Err(error);
        }
        Ok(())
    }

    fn save(&self, values: &Map<String, Value>) -> Result<()> {
        let Some(path) = &self.state.path else {
            return Ok(());
        };
        fs::create_dir_all(path.parent().context("host settings path has no parent")?)?;
        let temporary = path.with_extension("json.tmp");
        fs::write(&temporary, serde_json::to_vec_pretty(values)?)?;
        fs::rename(&temporary, path)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn cloned_settings_preserve_concurrent_updates_and_failed_write_state() -> Result<()> {
        #[cfg(target_os = "macos")]
        std::panic::set_hook(Box::new(|info| {
            eprintln!("{info}");
            std::process::exit(101);
        }));
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("settings.json");
        let mut settings = Settings::load(path.clone())?;
        settings.write("unrelated", json!("kept"))?;
        let threads: Vec<_> = ["editor", "terminal"]
            .into_iter()
            .map(|key| {
                let mut writer = settings.clone();
                std::thread::spawn(move || writer.write(key, json!(true)))
            })
            .collect();
        for thread in threads {
            thread.join().expect("settings writer")?;
        }
        let saved: Value = serde_json::from_slice(&fs::read(&path)?)?;
        assert_eq!(
            saved,
            json!({"editor": true, "terminal": true, "unrelated": "kept"})
        );
        assert_eq!(settings.read_all()["editor"], true);
        assert_eq!(settings.read_all()["terminal"], true);
        fs::remove_file(&path)?;
        fs::create_dir(&path)?;
        let mut writer = settings.clone();
        assert!(writer.write("editor", json!(false)).is_err());
        assert_eq!(settings.read_all()["editor"], true);
        Ok(())
    }
}
