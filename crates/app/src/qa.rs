use anyhow::{Context as _, Result, ensure};
use serde_json::json;
use std::{
    fs,
    os::unix::fs::PermissionsExt as _,
    path::{Path, PathBuf},
    process::Command,
    sync::OnceLock,
};

pub const ENABLED: bool = option_env!("CODEX_GPUI_QA").is_some();
pub const REVISION: &str = match option_env!("CODEX_GPUI_REVISION") {
    Some(revision) => revision,
    None => "development",
};

static PROFILE: OnceLock<PathBuf> = OnceLock::new();

pub fn profile() -> Option<&'static PathBuf> {
    PROFILE.get()
}

fn private_directory(path: &Path) -> Result<()> {
    let parent = path.parent().context("QA directory has no parent")?;
    ensure!(parent.canonicalize()? == parent, "QA parent is redirected");
    if !path.exists() {
        fs::create_dir(path)?;
    }
    ensure!(
        fs::symlink_metadata(path)?.file_type().is_dir(),
        "QA directory is not a real directory"
    );
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    Ok(())
}

pub fn prepare() -> Result<()> {
    if !ENABLED {
        return Ok(());
    }
    let home =
        PathBuf::from(std::env::var_os("HOME").context("HOME is not set")?).canonicalize()?;
    let root = home.join("Library/Application Support/codex-gpui-qa/profile");
    let root = match std::env::args().skip(1).collect::<Vec<_>>().as_slice() {
        [flag, path] if flag == "--qa-smoke-profile" => {
            let path = PathBuf::from(path).canonicalize()?;
            ensure!(
                path.starts_with(PathBuf::from("/tmp").canonicalize()?),
                "smoke profile must be under /tmp"
            );
            path
        }
        _ => {
            private_directory(root.parent().context("QA profile has no parent")?)?;
            root
        }
    };
    private_directory(&root)?;
    for directory in ["home", "codex", "workspace"] {
        private_directory(&root.join(directory))?;
    }
    // This runs before GPUI, app-server, or any worker thread starts.
    unsafe {
        std::env::set_var("HOME", root.join("home"));
        std::env::set_var("CODEX_HOME", root.join("codex"));
        std::env::set_var("CODEX_GPUI_CWD", root.join("workspace"));
    }
    PROFILE
        .set(root)
        .map_err(|_| anyhow::anyhow!("QA profile already initialized"))?;
    Ok(())
}

pub fn configure_command(command: &mut Command) {
    let Some(root) = profile() else { return };
    command
        .env_clear()
        .env("HOME", root.join("home"))
        .env("CODEX_HOME", root.join("codex"))
        .env(
            "PATH",
            "/usr/bin:/bin:/usr/sbin:/sbin:/opt/homebrew/bin:/opt/zerobrew/bin",
        )
        .env("TMPDIR", "/tmp")
        .args(["-c", "cli_auth_credentials_store=\"file\""]);
}

pub fn build_info() -> serde_json::Value {
    json!({"qa": ENABLED, "revision": REVISION})
}

pub fn monitor(handle: gpui::WindowHandle<crate::Root>, cx: &gpui::App) {
    let Some(profile) = profile() else { return };
    cx.spawn(async move |cx| {
        loop {
            cx.background_executor()
                .timer(std::time::Duration::from_secs(1))
                .await;
            match handle.update(cx, |root, window, cx| tick(profile, root, window, cx)) {
                Ok(Ok(())) => {}
                Ok(Err(error)) => eprintln!("codex-gpui QA status: {error:#}"),
                Err(_) => break,
            }
        }
    })
    .detach();
}

fn tick(
    profile: &std::path::Path,
    root: &mut crate::Root,
    window: &mut gpui::Window,
    cx: &mut gpui::Context<crate::Root>,
) -> Result<()> {
    let pid = std::process::id();
    let executable = std::env::current_exe()?.canonicalize()?;
    let ready = root.qa_restart_ready(window, cx);
    let status = json!({
        "pid": pid, "revision": REVISION, "executable": executable,
        "profile": profile, "ready": ready, "active": window.is_window_active(),
        "signed_out": root.signed_out(), "initialized": root.ready(),
        "time": std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_secs(),
    });
    let temporary = profile.join("runtime.json.tmp");
    fs::write(&temporary, serde_json::to_vec(&status)?)?;
    fs::rename(temporary, profile.join("runtime.json"))?;
    let request = profile.join("restart.json");
    if let Ok(bytes) = fs::read(&request) {
        let request_value: serde_json::Value = serde_json::from_slice(&bytes)?;
        if ready
            && request_value["pid"] == pid
            && request_value["revision"] == REVISION
            && request_value["executable"] == status["executable"]
        {
            root.save(window, cx);
            fs::remove_file(request)?;
            window.remove_window();
        }
    }
    Ok(())
}
