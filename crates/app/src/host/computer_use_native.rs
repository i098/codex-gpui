use anyhow::{Context as _, Result, bail, ensure};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::Mutex,
};

const HELPER: &str = "Codex Computer Use.app";
const EXECUTABLE: &str = "Contents/MacOS/SkyComputerUseService";
const INSTALLER: &str = "Contents/SharedSupport/Codex Computer Use Installer.app/Contents/MacOS/Codex Computer Use Installer";
const CAPTURE_DIRECTORY: &str = "com.openai.sky.CUAService";
const MAX_CAPTURE_BYTES: u64 = 25 * 1024 * 1024;

/// The original locked-use gates, supplied by feature and account requirements.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LockedUsePolicy {
    pub locked_use_enabled: bool,
    pub slingshot_enabled: bool,
    pub remote_control_available: Option<bool>,
    pub remote_control_access_required: bool,
    pub allow_locked_computer_use: Option<bool>,
}

impl LockedUsePolicy {
    pub fn allowed(&self) -> bool {
        self.locked_use_enabled
            && self.slingshot_enabled
            && self.remote_control_available != Some(false)
            && !self.remote_control_access_required
            && self.allow_locked_computer_use != Some(false)
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LockedUseState {
    pub enabled: Option<bool>,
    #[serde(rename = "computerIconDataURL")]
    pub computer_icon_data_url: Option<String>,
    #[serde(rename = "lockIconDataURL")]
    pub lock_icon_data_url: Option<String>,
}

impl LockedUseState {
    pub fn visible(&self, policy: &LockedUsePolicy) -> bool {
        policy.allowed() && self.enabled.is_some()
    }
}

/// One request owns all updates until completion, failure, or cancellation.
#[derive(Clone, Debug, Default)]
pub struct Appshot {
    pub request_id: String,
    pub app: Option<Value>,
    pub ax_text: Option<String>,
    pub screenshot: Option<Value>,
    pub completed: bool,
    pub failure_reason: Option<String>,
}

impl Appshot {
    pub fn new(request_id: String) -> Self {
        Self {
            request_id,
            ..Self::default()
        }
    }

    pub fn update(&mut self, request_id: &str, update: &Value) -> Result<(), &'static str> {
        if self.request_id != request_id || self.completed {
            return Err("capture request is stale");
        }
        match update["type"].as_str() {
            Some("metadata") if update["app"]["bundleIdentifier"].is_string() => {
                self.app = Some(update["app"].clone());
            }
            Some("axText") if update["text"].is_string() => {
                self.ax_text = update["text"].as_str().map(str::to_owned);
            }
            Some("screenshot") => self.screenshot = Some(update.clone()),
            Some("completed") => self.completed = true,
            Some("failed") => {
                self.completed = true;
                self.failure_reason = Some(
                    update["failureReason"]
                        .as_str()
                        .filter(|s| !s.trim().is_empty())
                        .unwrap_or("native_service_failed")
                        .to_owned(),
                );
            }
            _ => return Err("invalid native capture update"),
        }
        Ok(())
    }

    pub fn cancel(&mut self) {
        self.completed = true;
    }
}

#[derive(Debug)]
pub struct NativeReplyError {
    pub code: i32,
    pub message: String,
}

impl std::fmt::Display for NativeReplyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Codex Computer Use Apple Event error {}: {}",
            self.code, self.message
        )
    }
}

impl std::error::Error for NativeReplyError {}

#[derive(Default)]
struct ManagedHelper {
    enabled: bool,
    child: Option<std::process::Child>,
}

/// Owns only the helper it starts. Construction and status reads never start it.
pub struct NativeComputerUse {
    helper: PathBuf,
    capture_root: PathBuf,
    managed: Mutex<ManagedHelper>,
}

impl NativeComputerUse {
    pub fn new(codex_home: &Path) -> Self {
        Self {
            helper: codex_home.join("computer-use").join(HELPER),
            capture_root: std::env::temp_dir().join(CAPTURE_DIRECTORY),
            managed: Mutex::new(ManagedHelper::default()),
        }
    }

    pub fn canonical_helper_path(&self) -> &Path {
        &self.helper
    }

    pub fn installer_path(&self) -> PathBuf {
        self.helper.join(INSTALLER)
    }

    /// Copies the signed helper only after an explicit setup action.
    pub fn install_helper(&self, source: &Path) -> Result<()> {
        require_macos()?;
        let mut managed = self
            .managed
            .lock()
            .map_err(|_| anyhow::anyhow!("native helper lock poisoned"))?;
        ensure!(
            !managed.enabled,
            "disable the managed helper before installation"
        );
        if let Some(child) = managed.child.as_mut() {
            ensure!(
                child.try_wait()?.is_some(),
                "the managed helper is still running"
            );
        }
        verify_helper(source)?;
        if source.canonicalize()? == self.helper.canonicalize().unwrap_or_default() {
            return Ok(());
        }
        let parent = self.helper.parent().context("missing helper parent")?;
        fs::create_dir_all(parent)?;
        let stage = tempfile::tempdir_in(parent)?;
        let replacement = stage.path().join(HELPER);
        run(std::process::Command::new("/usr/bin/ditto")
            .arg("--noqtn")
            .arg(source)
            .arg(&replacement))?;
        verify_helper(&replacement)?;
        let previous = stage.path().join("previous.app");
        let had_previous = self.helper.try_exists()?;
        if had_previous {
            verify_helper(&self.helper)?;
            fs::rename(&self.helper, &previous)?;
        }
        if let Err(error) = fs::rename(&replacement, &self.helper) {
            if had_previous {
                fs::rename(&previous, &self.helper).context("restore previous signed helper")?;
            }
            return Err(error).context("install canonical helper");
        }
        Ok(())
    }

    /// Call only for an explicit user action; this can cause macOS permission prompts.
    pub fn enable(&self) -> Result<()> {
        require_macos()?;
        let mut managed = self
            .managed
            .lock()
            .map_err(|_| anyhow::anyhow!("native helper lock poisoned"))?;
        verify_helper(&self.helper)?;
        if let Some(child) = managed.child.as_mut()
            && child.try_wait()?.is_none()
        {
            #[cfg(target_os = "macos")]
            ensure!(
                native::matches_executable(child.id(), &self.helper.join(EXECUTABLE))?,
                "managed helper executable changed"
            );
            managed.enabled = true;
            return Ok(());
        }
        managed.child = Some(
            std::process::Command::new(self.helper.join(EXECUTABLE))
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
                .context("start managed Computer Use helper")?,
        );
        managed.enabled = true;
        Ok(())
    }

    pub fn disable(&self) -> Result<()> {
        self.managed
            .lock()
            .map_err(|_| anyhow::anyhow!("native helper lock poisoned"))?
            .enabled = false;
        // The original leaves the signed service alive; do not kill a shared native service.
        Ok(())
    }

    pub fn locked_use_state(&self) -> Result<LockedUseState> {
        if !cfg!(target_os = "macos") {
            return Ok(LockedUseState::default());
        }
        verify_helper(&self.helper)?;
        let output = run(std::process::Command::new(self.installer_path()).arg("status"))?;
        #[cfg(target_os = "macos")]
        let (computer_icon_data_url, lock_icon_data_url) = native::icons();
        #[cfg(not(target_os = "macos"))]
        let (computer_icon_data_url, lock_icon_data_url) = (None, None);
        Ok(LockedUseState {
            enabled: Some(output.trim() == "OK: installed"),
            computer_icon_data_url,
            lock_icon_data_url,
        })
    }

    pub fn set_locked_use_enabled(
        &self,
        enabled: bool,
        policy: &LockedUsePolicy,
    ) -> Result<LockedUseState> {
        require_macos()?;
        ensure!(
            policy.allowed(),
            "Locked use is not allowed by the current account policy"
        );
        verify_helper(&self.helper)?;
        run(
            std::process::Command::new(self.installer_path()).arg(if enabled {
                "install"
            } else {
                "uninstall"
            }),
        )?;
        self.locked_use_state()
    }

    pub fn request_permissions(&self, app: &str) -> Result<Value> {
        ensure!(!app.trim().is_empty(), "missing application identifier");
        match self.request(
            "ComputerUseIPCAppGetSkyshotRequest",
            &json!({"app": app}),
            115,
        ) {
            Ok(_) => Ok(json!("granted")),
            Err(error)
                if error
                    .downcast_ref::<NativeReplyError>()
                    .is_some_and(|e| e.code == -10014) =>
            {
                Ok(json!("pending"))
            }
            Err(error) => Err(error),
        }
    }

    pub fn start_capture(
        &self,
        app: &str,
        permission_request_id: &str,
        request_id: &str,
        animation_target: &Value,
    ) -> Result<Value> {
        ensure!(
            !app.trim().is_empty() && !permission_request_id.is_empty() && !request_id.is_empty(),
            "missing capture identity"
        );
        ensure!(
            animation_target.is_object(),
            "missing native capture animation target"
        );
        let response = self.request(
            "ComputerUseIPCAppStartCaptureRequest",
            &json!({
                "app": app, "permissionRequestId": permission_request_id, "requestId": request_id,
                "animationTarget": animation_target, "version": 2
            }),
            115,
        )?;
        ensure!(
            matches!(
                response["result"].as_str(),
                Some("started" | "appshot_permissions_abandoned")
            ),
            "unknown native capture start result"
        );
        Ok(response)
    }

    pub fn next_capture_update(&self, request_id: &str) -> Result<Value> {
        ensure!(!request_id.is_empty(), "missing capture request identity");
        let update = self.request(
            "ComputerUseIPCAppNextCaptureUpdateRequest",
            &json!({"requestId": request_id}),
            125,
        )?;
        capture_update(&self.capture_root, update)
    }

    pub fn stop_application(&self, app: &str) -> Result<()> {
        ensure!(!app.trim().is_empty(), "missing application identifier");
        self.request("ComputerUseIPCAppStopRequest", &json!({"app": app}), 15)?;
        Ok(())
    }

    pub fn request(
        &self,
        request_type: &str,
        payload: &Value,
        timeout_seconds: u32,
    ) -> Result<Value> {
        require_macos()?;
        ensure!(
            (1..=125).contains(&timeout_seconds),
            "invalid native request timeout"
        );
        let mut managed = self
            .managed
            .lock()
            .map_err(|_| anyhow::anyhow!("native helper lock poisoned"))?;
        ensure!(managed.enabled, "managed Computer Use helper is disabled");
        let child = managed
            .child
            .as_mut()
            .context("managed Computer Use helper has not started")?;
        ensure!(
            child.try_wait()?.is_none(),
            "managed Computer Use helper exited; enable it again"
        );
        let pid = child.id();
        drop(managed);
        #[cfg(target_os = "macos")]
        {
            ensure!(
                native::matches_executable(pid, &self.helper.join(EXECUTABLE))?,
                "managed helper executable changed"
            );
            native::request(pid, request_type, payload, timeout_seconds)
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = (pid, request_type, payload);
            bail!("Computer Use native IPC requires macOS")
        }
    }
}

fn require_macos() -> Result<()> {
    ensure!(
        cfg!(target_os = "macos"),
        "Computer Use native integration requires macOS"
    );
    Ok(())
}

fn verify_helper(helper: &Path) -> Result<()> {
    ensure!(
        helper.join(EXECUTABLE).is_file() && helper.join(INSTALLER).is_file(),
        "signed Computer Use helper is incomplete"
    );
    run(std::process::Command::new("/usr/bin/codesign").args(["--verify", "--deep", "--strict", "-R", "=anchor apple generic and identifier \"com.openai.sky.CUAService\" and certificate leaf[subject.OU] = \"2DC432GLL2\""]).arg(helper))?;
    Ok(())
}

fn run(command: &mut std::process::Command) -> Result<String> {
    use std::{
        io::{Read as _, Seek as _},
        process::Stdio,
        time::{Duration, Instant},
    };
    let mut stdout = tempfile::tempfile()?;
    let mut stderr = tempfile::tempfile()?;
    let mut child = command
        .stdin(Stdio::null())
        .stdout(stdout.try_clone()?)
        .stderr(stderr.try_clone()?)
        .spawn()?;
    let started = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if started.elapsed() >= Duration::from_secs(120) {
            child.kill()?;
            child.wait()?;
            bail!("native setup command timed out after 120 seconds");
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    let mut text = String::new();
    if !status.success() {
        stderr.rewind()?;
        stderr.take(65536).read_to_string(&mut text)?;
        bail!("native setup command failed ({status}): {}", text.trim());
    }
    stdout.rewind()?;
    stdout.take(65536).read_to_string(&mut text)?;
    Ok(text)
}

pub fn capture_update(capture_root: &Path, mut update: Value) -> Result<Value> {
    match update["type"].as_str() {
        Some("screenshot") => {
            let screenshot = update["screenshotURL"]
                .as_str()
                .or_else(|| update["screenshot"]["url"].as_str());
            let image = capture_image(capture_root, screenshot)?;
            let transition = capture_image(capture_root, update["transitionSnapshotURL"].as_str())?;
            update = json!({"type": "screenshot", "screenshotPath": image.0, "screenshotDataURL": image.1});
            if let (Some(path), data) = transition {
                update["transitionSnapshotPath"] = json!(path);
                update["transitionSnapshotDataURL"] = json!(data);
            }
        }
        Some("completed") => {
            let transition = capture_image(capture_root, update["transitionSnapshotURL"].as_str())?;
            update = json!({"type": "completed", "transitionSnapshotPath": transition.0, "transitionSnapshotDataURL": transition.1});
        }
        Some("metadata") => ensure!(
            update["app"]["bundleIdentifier"].is_string(),
            "invalid capture app metadata"
        ),
        Some("axText") => ensure!(
            update["text"].is_string(),
            "invalid capture accessibility text"
        ),
        Some("failed") => {
            if update["failureReason"]
                .as_str()
                .is_none_or(|s| s.trim().is_empty())
            {
                update["failureReason"] = json!("native_service_failed");
            }
        }
        _ => bail!("unknown native capture update"),
    }
    Ok(update)
}

fn capture_image(root: &Path, source: Option<&str>) -> Result<(Option<String>, Option<String>)> {
    use std::io::Read as _;
    let Some(source) = source.filter(|s| !s.trim().is_empty()) else {
        return Ok((None, None));
    };
    let path = if source.starts_with("file:") {
        url::Url::parse(source)?
            .to_file_path()
            .map_err(|_| anyhow::anyhow!("invalid capture file URL"))?
    } else {
        PathBuf::from(source)
    };
    ensure!(path.is_absolute(), "capture path is not absolute");
    let path = path.canonicalize().context("resolve capture image")?;
    ensure!(
        path.starts_with(root.canonicalize()?),
        "capture image is outside the helper capture directory"
    );
    let mime = match path
        .extension()
        .and_then(|v| v.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        _ => bail!("unsupported capture image type"),
    };
    let file = fs::File::open(&path)?;
    let metadata = file.metadata()?;
    ensure!(
        metadata.is_file() && metadata.len() <= MAX_CAPTURE_BYTES,
        "capture image exceeds the 25 MiB limit or is not a file"
    );
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    file.take(MAX_CAPTURE_BYTES + 1).read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() as u64 <= MAX_CAPTURE_BYTES,
        "capture image exceeds the 25 MiB limit"
    );
    Ok((
        Some(path.to_string_lossy().into_owned()),
        Some(format!("data:{mime};base64,{}", STANDARD.encode(bytes))),
    ))
}

#[cfg(target_os = "macos")]
mod native {
    use super::*;
    use std::ffi::{CStr, c_void};

    pub fn icons() -> (Option<String>, Option<String>) {
        use objc2::{AnyThread as _, rc::autoreleasepool};
        use objc2_app_kit::NSImage;
        use objc2_foundation::NSString;

        autoreleasepool(|_| {
            let computer = NSImage::imageNamed(&NSString::from_str("NSComputer"));
            let lock = NSImage::initWithContentsOfFile(
                NSImage::alloc(),
                &NSString::from_str(
                    "/System/Library/CoreServices/CoreTypes.bundle/Contents/Resources/LockedIcon.icns",
                ),
            );
            (
                computer.as_deref().and_then(png_data_url),
                lock.as_deref().and_then(png_data_url),
            )
        })
    }

    fn png_data_url(image: &objc2_app_kit::NSImage) -> Option<String> {
        use objc2_app_kit::{NSBitmapImageFileType, NSBitmapImageRep};
        use objc2_foundation::NSDictionary;

        let tiff = image.TIFFRepresentation()?;
        let bitmap = NSBitmapImageRep::imageRepWithData(&tiff)?;
        // The empty dictionary has no property values with an incompatible Objective-C type.
        let png = unsafe {
            bitmap.representationUsingType_properties(
                NSBitmapImageFileType::PNG,
                &NSDictionary::new(),
            )
        }?;
        // The retained NSData stays immutable while its bytes are encoded.
        let bytes = unsafe { png.as_bytes_unchecked() };
        Some(format!("data:image/png;base64,{}", STANDARD.encode(bytes)))
    }

    #[repr(C, packed(2))]
    #[derive(Default)]
    struct Descriptor {
        kind: u32,
        data: *mut c_void,
    }

    #[link(name = "Carbon", kind = "framework")]
    unsafe extern "C" {
        fn AECreateDesc(
            kind: u32,
            data: *const c_void,
            size: isize,
            result: *mut Descriptor,
        ) -> i32;
        fn AECreateAppleEvent(
            class: u32,
            id: u32,
            target: *const Descriptor,
            return_id: i16,
            transaction: i32,
            result: *mut Descriptor,
        ) -> i32;
        fn AEPutParamPtr(
            event: *mut Descriptor,
            keyword: u32,
            kind: u32,
            data: *const c_void,
            size: isize,
        ) -> i32;
        fn AESendMessage(
            event: *const Descriptor,
            reply: *mut Descriptor,
            mode: i32,
            timeout: i32,
        ) -> i32;
        fn AEGetParamDesc(
            event: *const Descriptor,
            keyword: u32,
            desired: u32,
            result: *mut Descriptor,
        ) -> i32;
        fn AEGetDescDataSize(descriptor: *const Descriptor) -> isize;
        fn AEGetDescData(descriptor: *const Descriptor, data: *mut c_void, size: isize) -> i32;
        fn AEDisposeDesc(descriptor: *mut Descriptor) -> i32;
    }
    unsafe extern "C" {
        fn proc_pidpath(pid: i32, buffer: *mut c_void, size: u32) -> i32;
    }

    impl Drop for Descriptor {
        fn drop(&mut self) {
            unsafe {
                AEDisposeDesc(self);
            }
        }
    }

    const fn code(value: &[u8; 4]) -> u32 {
        u32::from_be_bytes(*value)
    }

    fn check(status: i32, operation: &str) -> Result<()> {
        ensure!(status == 0, "{operation} failed with {status}");
        Ok(())
    }

    pub fn matches_executable(pid: u32, executable: &Path) -> Result<bool> {
        let mut buffer = [0u8; 4096];
        let size =
            unsafe { proc_pidpath(pid as i32, buffer.as_mut_ptr().cast(), buffer.len() as u32) };
        ensure!(size > 0, "cannot inspect managed helper executable");
        let actual = CStr::from_bytes_until_nul(&buffer)?.to_str()?;
        Ok(Path::new(actual).canonicalize()? == executable.canonicalize()?)
    }

    fn put(event: &mut Descriptor, key: &[u8; 4], kind: &[u8; 4], data: &[u8]) -> Result<()> {
        check(
            unsafe {
                AEPutParamPtr(
                    event,
                    code(key),
                    code(kind),
                    data.as_ptr().cast(),
                    data.len() as isize,
                )
            },
            "AEPutParamPtr",
        )
    }

    fn get(event: &Descriptor, key: &[u8; 4], kind: &[u8; 4]) -> Result<Vec<u8>> {
        let mut descriptor = Descriptor::default();
        check(
            unsafe { AEGetParamDesc(event, code(key), code(kind), &mut descriptor) },
            "AEGetParamDesc",
        )?;
        let size = unsafe { AEGetDescDataSize(&descriptor) };
        ensure!(
            (0..=1024 * 1024).contains(&size),
            "native Apple Event response exceeds 1 MiB"
        );
        let mut bytes = vec![0; size as usize];
        check(
            unsafe { AEGetDescData(&descriptor, bytes.as_mut_ptr().cast(), size) },
            "AEGetDescData",
        )?;
        Ok(bytes)
    }

    pub fn request(pid: u32, request_type: &str, payload: &Value, timeout: u32) -> Result<Value> {
        let mut target = Descriptor::default();
        let mut event = Descriptor::default();
        let mut reply = Descriptor::default();
        let pid = pid as i32;
        check(
            unsafe { AECreateDesc(code(b"kpid"), (&pid as *const i32).cast(), 4, &mut target) },
            "AECreateDesc",
        )?;
        check(
            unsafe { AECreateAppleEvent(code(b"SkCu"), code(b"SndR"), &target, -1, 0, &mut event) },
            "AECreateAppleEvent",
        )?;
        put(&mut event, b"RspT", b"utf8", request_type.as_bytes())?;
        put(
            &mut event,
            b"ClVn",
            b"utf8",
            b"CodexComputerUseNativeBridge-1",
        )?;
        put(&mut event, b"ReqD", b"tdta", &serde_json::to_vec(payload)?)?;
        for delay in [50, 100, 200, 400, 800, 0] {
            let status = unsafe { AESendMessage(&event, &mut reply, 3, timeout as i32 * 60) };
            if !matches!(status, -600 | -609 | -1708 | -1717) || delay == 0 {
                check(status, "AESendMessage")?;
                break;
            }
            reply = Descriptor::default();
            std::thread::sleep(std::time::Duration::from_millis(delay));
        }
        if let Ok(number) = get(&reply, b"errn", b"long") {
            let number = i32::from_ne_bytes(
                number
                    .try_into()
                    .map_err(|_| anyhow::anyhow!("invalid native error number"))?,
            );
            let text = get(&reply, b"errs", b"utf8")
                .map(|v| String::from_utf8_lossy(&v).into_owned())
                .unwrap_or_else(|_| "Unknown error".into());
            return Err(NativeReplyError {
                code: number,
                message: text,
            }
            .into());
        }
        serde_json::from_slice(&get(&reply, b"----", b"tdta")?)
            .context("decode native Computer Use reply")
    }
}
