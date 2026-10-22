#[cfg(not(target_os = "macos"))]
fn main() {}

#[cfg(target_os = "macos")]
fn main() -> anyhow::Result<()> {
    std::panic::set_hook(Box::new(|info| {
        eprintln!("{info}");
        std::process::exit(101);
    }));
    use gpui::{AppContext as _, Bounds, WindowBounds, WindowOptions, px, size};
    use objc2::{class, msg_send, runtime::AnyObject, sel};
    use objc2_foundation::{NSPoint, NSString};
    use std::{ffi::c_void, path::PathBuf, time::Duration};
    enum CGImage {}
    unsafe impl objc2::encode::RefEncode for CGImage {
        const ENCODING_REF: objc2::encode::Encoding = objc2::encode::Encoding::Pointer(
            &objc2::encode::Encoding::Struct("CGImage", &[]),
        );
    }

    #[link(name = "CoreGraphics", kind = "framework")]
    unsafe extern "C" {
        fn CGWindowListCopyWindowInfo(options: u32, relative: u32) -> *mut AnyObject;
        fn CGWindowListCreateImage(bounds: objc2_foundation::NSRect, options: u32, window: u32, image_options: u32) -> *mut CGImage;
    }
    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" { fn CFRelease(object: *const c_void); }

    // One File popup per reservation: refuse to start if the attempt marker already exists.
    let out = PathBuf::from(std::env::var("MENU_CAPTURE_OUT")?);
    std::fs::create_dir_all(&out)?;
    std::fs::OpenOptions::new().write(true).create_new(true).open(out.join("attempt-1.reserved"))?;
    // Bounded exit even if the popup or capture hangs.
    std::thread::spawn(|| {
        std::thread::sleep(Duration::from_secs(20));
        eprintln!("file-menu: watchdog exit");
        std::process::exit(102);
    });
    // Owned capture workspace; the runner sets HOME/CODEX_HOME/TMPDIR under the same owned root.
    let workspace = std::env::var("MENU_CAPTURE_WORKSPACE")?;
    let recording = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/readback.jsonl");
    let (bridge, events) = bridge::Bridge::replay(&recording)?;
    // Observe the frontmost app without changing focus.
    fn frontmost(label: &str) {
        unsafe {
            let workspace: *mut AnyObject = msg_send![class!(NSWorkspace), sharedWorkspace];
            let app: *mut AnyObject = msg_send![workspace, frontmostApplication];
            if app.is_null() { println!("{label}: none"); return; }
            let pid: i32 = msg_send![app, processIdentifier];
            let bundle: *mut AnyObject = msg_send![app, bundleIdentifier];
            let bundle = if bundle.is_null() { String::new() } else {
                let utf8: *const std::ffi::c_char = msg_send![bundle, UTF8String];
                std::ffi::CStr::from_ptr(utf8).to_string_lossy().into_owned()
            };
            println!("{label}: pid={pid} bundle={bundle}");
        }
    }
    gpui_platform::application().with_assets(theme::Assets).run(move |cx| {
        theme::load_fonts(cx);
        cx.set_global(theme::Theme::dark());
        app::init(cx);
        frontmost("foreground_before_activation");
        cx.set_menus(app::menus());
        let root = cx.open_window(WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds::centered(None, size(px(1280.), px(832.)), cx))),
            ..Default::default()
        }, |window, cx| cx.new(|cx| app::Root::new(app::State::default(), bridge, events, workspace, window, cx))).expect("open owned fixture window");
        cx.activate(true);
        cx.spawn(async move |cx| {
            cx.background_executor().timer(Duration::from_secs(2)).await;
            {
                let (title, name) = ("File", "file");
                let file = out.join(format!("{name}.png"));
                let capture = std::thread::spawn(move || {
                    std::thread::sleep(Duration::from_millis(500));
                    unsafe {
                        let list = CGWindowListCopyWindowInfo(1, 0);
                        if list.is_null() { return false; }
                        let count: usize = msg_send![list, count];
                        let mut captured = false;
                        // Capture at most one owned popup window (first owned nonzero layer).
                        for position in 0..count {
                            let row: *mut AnyObject = msg_send![list, objectAtIndex: position];
                            let owner: *mut AnyObject = msg_send![row, objectForKey: &*NSString::from_str("kCGWindowOwnerPID")];
                            let pid: u32 = msg_send![owner, unsignedIntValue];
                            let layer: *mut AnyObject = msg_send![row, objectForKey: &*NSString::from_str("kCGWindowLayer")];
                            let layer: i32 = msg_send![layer, intValue];
                            if pid != std::process::id() || layer == 0 { continue; }
                            let number: *mut AnyObject = msg_send![row, objectForKey: &*NSString::from_str("kCGWindowNumber")];
                            let number: u32 = msg_send![number, unsignedIntValue];
                            let bounds = objc2_foundation::NSRect::new(NSPoint::new(f64::INFINITY, f64::INFINITY), objc2_foundation::NSSize::new(0., 0.));
                            let image = CGWindowListCreateImage(bounds, 8, number, 1);
                            if image.is_null() { println!("file: create_image=null window={number}"); break; }
                            let bitmap: *mut AnyObject = msg_send![class!(NSBitmapImageRep), alloc];
                            let bitmap: *mut AnyObject = msg_send![bitmap, initWithCGImage: image];
                            let properties: *mut AnyObject = msg_send![class!(NSDictionary), dictionary];
                            let data: *mut AnyObject = msg_send![bitmap, representationUsingType: 4usize, properties: properties];
                            let ok: bool = msg_send![data, writeToFile: &*NSString::from_str(&file.to_string_lossy()), atomically: true];
                            let _: () = msg_send![bitmap, release];
                            CFRelease(image.cast());
                            captured = ok;
                            println!("file: window={number} layer={layer}");
                            break;
                        }
                        CFRelease(list.cast());
                        captured
                    }
                });
                let menu = root.update(cx, |_, _, _| unsafe {
                    let application: *mut AnyObject = msg_send![class!(NSApplication), sharedApplication];
                    let active: bool = msg_send![application, isActive];
                    println!("pid={} app_active_before_popup={active}", std::process::id());
                    let bar: *mut AnyObject = msg_send![application, mainMenu];
                    let item: *mut AnyObject = msg_send![bar, itemWithTitle: &*NSString::from_str(title)];
                    let menu: *mut AnyObject = msg_send![item, submenu];
                    menu as usize
                }).expect("read native menu") as *mut AnyObject;
                println!("{name}: menu_found={}", !menu.is_null());
                if !menu.is_null() {
                    unsafe {
                        let _: () = msg_send![menu, performSelector: sel!(cancelTracking), withObject: std::ptr::null::<AnyObject>(), afterDelay: 1.5f64];
                        let _: bool = msg_send![menu, popUpMenuPositioningItem: std::ptr::null::<AnyObject>(), atLocation: NSPoint::new(100., 600.), inView: std::ptr::null::<AnyObject>()];
                    }
                }
                println!("{name}: captured={}", capture.join().expect("capture worker"));
            }
            frontmost("foreground_after_capture");
            cx.update(|cx| cx.quit());
        }).detach();
    });
    Ok(())
}
