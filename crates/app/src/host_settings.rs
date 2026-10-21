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
        let mut values = self.read_all();
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
