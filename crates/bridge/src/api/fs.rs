//! `fs/*` requests on the app-server host's file system.

use anyhow::{Result, anyhow};
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use serde_json::{Value, json};

use crate::Bridge;

impl Bridge {
    /// `fs/readFile`: the file's bytes.
    pub async fn fs_read_file(&self, path: &str) -> Result<Vec<u8>> {
        let file: Value = self.request("fs/readFile", json!({ "path": path })).await?;
        let data = file["dataBase64"]
            .as_str()
            .ok_or_else(|| anyhow!("fs/readFile: no dataBase64"))?;
        Ok(BASE64.decode(data)?)
    }

    /// `fs/createDirectory` with parents.
    pub async fn fs_create_directory(&self, path: &str) -> Result<()> {
        self.request::<_, Value>(
            "fs/createDirectory",
            json!({ "path": path, "recursive": true }),
        )
        .await?;
        Ok(())
    }

    pub async fn fs_write_file(&self, path: &str, bytes: &[u8]) -> Result<()> {
        let data = BASE64.encode(bytes);
        self.request::<_, Value>("fs/writeFile", json!({ "path": path, "dataBase64": data }))
            .await?;
        Ok(())
    }
}
