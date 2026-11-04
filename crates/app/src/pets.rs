//! Custom pets (`customAvatars.load`) and the Settings ▸ Mini & Pets actions: Refresh and
//! Create pet (install the bundled Hatch Pet skill, enable it, and prefill a new chat).

use crate::Root;
use anyhow::{Context as _, Result, ensure};
use gpui::{App, AppContext as _, Context, Window};
use serde_json::{Value, json};
use std::path::{Component, Path, PathBuf};
use ui::{
    screens::mini::{CustomPet, CustomPets, MiniOpen},
    toast::Toast,
};

/// Runs once per app: every window's `Root` calls it.
pub(crate) fn install(cx: &mut App) {
    if cx.has_global::<CustomPets>() {
        return;
    }
    cx.set_global(CustomPets::default());
    cx.set_global(MiniOpen::default());
    reload(cx);
}

/// Rereads `$CODEX_HOME/pets/*/pet.json` and the legacy `avatars/*/avatar.json`.
pub(crate) fn reload(cx: &mut App) {
    cx.global_mut::<CustomPets>().loading = true;
    let task = cx.background_spawn(async move { load(&codex_home()) });
    cx.spawn(async move |cx| {
        let result = task.await;
        cx.update(|cx| {
            ui::screens::mini::forget_custom();
            let pets = cx.global_mut::<CustomPets>();
            pets.loading = false;
            match result {
                Ok(list) => {
                    pets.pets = list;
                    pets.failed = false;
                }
                Err(error) => {
                    eprintln!("codex-gpui: custom pets: {error:#}");
                    pets.failed = true;
                }
            }
            cx.refresh_windows();
        });
    })
    .detach();
}

fn codex_home() -> PathBuf {
    std::env::var_os("CODEX_HOME")
        .filter(|home| !home.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".codex")
        })
}

/// The custom pets under `home`, newest manifest first; a `pets/` entry replaces the legacy
/// `avatars/` entry of the same id. Unreadable manifests are skipped.
fn load(home: &Path) -> Result<Vec<CustomPet>> {
    let mut found: Vec<(std::time::SystemTime, CustomPet)> = Vec::new();
    for (folder, manifest) in [("avatars", "avatar.json"), ("pets", "pet.json")] {
        let root = home.join(folder);
        let entries = match std::fs::read_dir(&root) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => {
                return Err(error).with_context(|| format!("Cannot read {}", root.display()));
            }
        };
        for entry in entries.flatten() {
            let directory = entry.path();
            let Some(name) = directory.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            let path = directory.join(manifest);
            let Ok((updated, pet)) = parse(&path, &directory, name) else {
                continue;
            };
            found.retain(|(_, existing)| existing.id != pet.id);
            found.push((updated, pet));
        }
    }
    found.sort_by_key(|(updated, _)| std::cmp::Reverse(*updated));
    Ok(found.into_iter().map(|(_, pet)| pet).collect())
}

fn parse(path: &Path, directory: &Path, name: &str) -> Result<(std::time::SystemTime, CustomPet)> {
    let manifest: Value = serde_json::from_slice(&std::fs::read(path)?)?;
    let display = manifest["displayName"]
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .context("pet has no displayName")?;
    let sheet = manifest["spritesheetPath"]
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("spritesheet.webp");
    ensure!(
        Path::new(sheet)
            .components()
            .all(|c| matches!(c, Component::Normal(_))),
        "spritesheet escapes the pet folder"
    );
    let updated = std::fs::metadata(path)?.modified()?;
    Ok((
        updated,
        CustomPet {
            id: format!("custom:{name}"),
            name: display.to_owned(),
            description: manifest["description"]
                .as_str()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_owned),
            sheet: directory.join(sheet),
            directory: directory.to_owned(),
        },
    ))
}

impl Root {
    /// `j`: install Hatch Pet, enable it, then open a new chat prefilled with its mention.
    pub(crate) fn create_pet(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if cx.global::<CustomPets>().creating {
            return;
        }
        cx.global_mut::<CustomPets>().creating = true;
        cx.refresh_windows();
        let host = self.host.clone();
        let bridge = self.bridge.clone();
        let task = cx.background_spawn(async move {
            let installed = crate::host::json_body(&host.fetch(&crate::host::Request::post(
                "install-recommended-skill",
                Some(json!({
                    "forceReinstall": true,
                    "installRoot": null,
                    "repoPath": "skills/.curated/hatch-pet",
                    "skillId": "hatch-pet",
                    "source": "bundled",
                })),
            ))?)?;
            let destination = installed["destination"]
                .as_str()
                .filter(|_| installed["success"] == true)
                .context("Unable to install Hatch Pet")?;
            let skill = Path::new(destination).join("SKILL.md");
            let written: Value = bridge
                .request(
                    "skills/config/write",
                    json!({"path": skill, "enabled": true}),
                )
                .await?;
            ensure!(
                written["effectiveEnabled"] != false,
                "Hatch Pet is disabled"
            );
            Ok::<_, anyhow::Error>(skill)
        });
        let window_handle = window.window_handle();
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = cx.update_window(window_handle, |_, window, cx| {
                cx.global_mut::<CustomPets>().creating = false;
                cx.refresh_windows();
                let _ = this.update(cx, |this, cx| match result {
                    Ok(skill) => this.plugin_prefill(
                        &format!(
                            "[$hatch-pet]({}) create a pet based on what you know about me",
                            skill.display()
                        ),
                        window,
                        cx,
                    ),
                    Err(error) => {
                        eprintln!("codex-gpui: create pet: {error:#}");
                        this.toast(Toast::danger("Unable to start pet creation"), cx);
                    }
                });
            });
        })
        .detach();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_pets_newest_first_and_skips_bad_manifests() -> Result<()> {
        let home = tempfile::tempdir()?;
        let write = |folder: &str, dir: &str, file: &str, body: &str| -> Result<()> {
            let path = home.path().join(folder).join(dir);
            std::fs::create_dir_all(&path)?;
            std::fs::write(path.join(file), body)?;
            Ok(())
        };
        write("avatars", "old", "avatar.json", r#"{"displayName":"Old"}"#)?;
        std::thread::sleep(std::time::Duration::from_millis(20));
        write(
            "pets",
            "new",
            "pet.json",
            r#"{"displayName":" New ","description":"Fresh","spritesheetPath":"s.webp"}"#,
        )?;
        write(
            "pets",
            "escape",
            "pet.json",
            r#"{"displayName":"X","spritesheetPath":"../x"}"#,
        )?;
        write(
            "pets",
            "nameless",
            "pet.json",
            r#"{"description":"no name"}"#,
        )?;
        let pets = load(home.path())?;
        let ids: Vec<_> = pets.iter().map(|p| p.id.as_str()).collect();
        assert_eq!(ids, ["custom:new", "custom:old"]);
        assert_eq!(pets[0].name, "New");
        assert_eq!(pets[0].description.as_deref(), Some("Fresh"));
        assert_eq!(pets[0].sheet, home.path().join("pets/new/s.webp"));
        assert_eq!(
            pets[1].sheet,
            home.path().join("avatars/old/spritesheet.webp")
        );
        Ok(())
    }
}
