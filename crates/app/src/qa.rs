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

struct Profile {
    root: PathBuf,
    _lock: fs::File,
}

static PROFILE: OnceLock<Profile> = OnceLock::new();

pub fn profile() -> Option<&'static PathBuf> {
    PROFILE.get().map(|profile| &profile.root)
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
                path.parent() == Some(PathBuf::from("/tmp").canonicalize()?.as_path())
                    && path
                        .file_name()
                        .is_some_and(|name| name.to_string_lossy().starts_with("cgx-qa-smoke-")),
                "smoke profile must be an owned cgx-qa-smoke-* directory directly under /tmp"
            );
            path
        }
        _ => {
            private_directory(root.parent().context("QA profile has no parent")?)?;
            root
        }
    };
    private_directory(&root)?;
    let lock = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(root.join("session.lock"))?;
    lock.try_lock()
        .context("QA profile is already open or updating")?;
    for directory in ["home", "codex", "workspace"] {
        private_directory(&root.join(directory))?;
    }
    // This runs before GPUI, app-server, or any worker thread starts.
    unsafe {
        std::env::set_var("HOME", root.join("home"));
        std::env::set_var("CODEX_HOME", root.join("codex"));
        std::env::set_var("CODEX_GPUI_CWD", root.join("workspace"));
        std::env::set_var("TMPDIR", "/tmp");
    }
    PROFILE
        .set(Profile { root, _lock: lock })
        .map_err(|_| anyhow::anyhow!("QA profile already initialized"))?;
    Ok(())
}

pub fn configure_command(command: &mut Command) -> Result<()> {
    let Some(root) = profile() else { return Ok(()) };
    let program = Path::new(command.get_program());
    let executable = if program.is_absolute() {
        program.to_owned()
    } else if program.components().count() > 1 {
        std::env::current_dir()?.join(program)
    } else {
        std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
            .map(|directory| directory.join(program))
            .find(|path| {
                fs::metadata(path).is_ok_and(|metadata| {
                    metadata.is_file() && metadata.permissions().mode() & 0o111 != 0
                })
            })
            .context("cannot resolve the QA app-server executable on the original PATH")?
    };
    let mut isolated = Command::new(executable.canonicalize()?);
    isolated
        .args(command.get_args())
        .env_clear()
        .env("HOME", root.join("home"))
        .env("CODEX_HOME", root.join("codex"))
        .env(
            "PATH",
            "/usr/bin:/bin:/usr/sbin:/sbin:/usr/local/bin:/opt/homebrew/bin:/opt/zerobrew/bin",
        )
        .env("TMPDIR", "/tmp")
        .args(["-c", "cli_auth_credentials_store=\"file\""]);
    *command = isolated;
    Ok(())
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
