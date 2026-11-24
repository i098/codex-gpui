use std::path::{Path, PathBuf};

pub(super) struct Destination {
    staging: tempfile::TempDir,
    path: PathBuf,
}

impl Destination {
    pub(super) fn new(directory: &Path, suggested: &str) -> std::io::Result<Self> {
        let name = Path::new(suggested)
            .file_name()
            .filter(|name| !name.as_encoded_bytes().contains(&0))
            .unwrap_or_else(|| "download".as_ref());
        std::fs::create_dir_all(directory)?;
        let staging = tempfile::Builder::new()
            .prefix(".codex-download-")
            .tempdir_in(directory)?;
        let path = staging.path().join(name);
        Ok(Self { staging, path })
    }

    pub(super) fn staging_path(&self) -> &Path {
        &self.path
    }

    pub(super) fn finish(self) -> std::io::Result<PathBuf> {
        let directory = self.staging.path().parent().expect("staging parent");
        let file_name = self.path.file_name().expect("download filename");
        let stem = self.path.file_stem().expect("download filename stem");
        let mut destination = directory.join(file_name);
        let mut suffix = 1;
        loop {
            // Publish only a completed file, without replacing an existing file or symlink.
            match std::fs::hard_link(&self.path, &destination) {
                Ok(()) => return Ok(destination),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    let mut name = stem.to_os_string();
                    name.push(format!(" ({suffix})"));
                    if let Some(extension) = self.path.extension() {
                        name.push(".");
                        name.push(extension);
                    }
                    destination.set_file_name(name);
                    suffix += 1;
                }
                Err(error) => return Err(error),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn download_keeps_existing_file_and_stays_in_downloads() -> anyhow::Result<()> {
        let directory = tempfile::tempdir()?;
        let original = directory.path().join("report.txt");
        std::fs::write(&original, "original")?;
        let download = Destination::new(directory.path(), "../report.txt")?;
        std::fs::write(download.staging_path(), "download")?;
        let saved: PathBuf = download.finish()?;
        assert_eq!(saved, directory.path().join("report (1).txt"));
        assert_eq!(std::fs::read_to_string(saved)?, "download");
        assert_eq!(std::fs::read_to_string(original)?, "original");
        Ok(())
    }
}
