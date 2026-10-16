use anyhow::{Context as _, Result};
use gpui::Global;
use serde_json::{Map, Value};
use std::{fs, io::ErrorKind, path::PathBuf};

/// The desktop host's `settings.readAll` and `settings.write(key, value)` service.
/// Host settings never go to the app-server's `config/value/write` endpoint.
#[derive(Default)]
pub struct Settings {
    values: Map<String, Value>,
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
            values,
            path: Some(path),
        })
    }

    pub fn read_all(&self) -> &Map<String, Value> {
        &self.values
    }

    pub fn write(&mut self, key: &str, value: Value) -> Result<()> {
        let old = self.values.insert(key.to_owned(), value);
        if let Err(error) = self.save() {
            match old {
                Some(value) => {
                    self.values.insert(key.to_owned(), value);
                }
                None => {
                    self.values.remove(key);
                }
            }
            return Err(error);
        }
        Ok(())
    }

    fn save(&self) -> Result<()> {
        let Some(path) = &self.path else {
            return Ok(());
        };
        fs::create_dir_all(path.parent().context("host settings path has no parent")?)?;
        let temporary = path.with_extension("json.tmp");
        fs::write(&temporary, serde_json::to_vec_pretty(&self.values)?)?;
        fs::rename(&temporary, path)?;
        Ok(())
    }
}
