#[cfg(target_os = "macos")]
#[path = "../src/updater/native.rs"]
mod native;

#[cfg(target_os = "macos")]
fn validate(feed: &str, _key: &str) -> anyhow::Result<()> {
    anyhow::ensure!(
        feed.starts_with("http://127.0.0.1:"),
        "local smoke feed only"
    );
    Ok(())
}

#[cfg(target_os = "macos")]
fn main() -> anyhow::Result<()> {
    use block2::RcBlock;
    use objc2::{class, msg_send, rc::Retained, runtime::AnyObject};
    use objc2_foundation::{NSBundle, NSString};
    use std::time::{Duration, Instant};
    use ui::updater::Status;

    let app: *mut AnyObject = unsafe { msg_send![class!(NSApplication), sharedApplication] };
    let _: bool = unsafe { msg_send![app, setActivationPolicy: 2_isize] };
    let version = NSBundle::mainBundle()
        .objectForInfoDictionaryKey(&NSString::from_str("CFBundleVersion"))
        .unwrap()
        .downcast::<NSString>()
        .unwrap()
        .to_string();
    if version == "2" {
        let marker = std::path::PathBuf::from(NSBundle::mainBundle().bundlePath().to_string())
            .with_file_name("relaunched");
        std::fs::write(marker, "version=2\n")?;
        return Ok(());
    }
    let (backend, events) = native::start()?.expect("configured native updater");
    backend.check();
    let deadline = Instant::now() + Duration::from_secs(90);
    let canceled_at = std::cell::Cell::new(None);
    let poll = RcBlock::new(move |_timer: *mut AnyObject| {
        if canceled_at
            .get()
            .is_some_and(|at: Instant| at.elapsed() >= Duration::from_secs(1))
        {
            std::process::exit(0);
        }
        if Instant::now() >= deadline {
            eprintln!("native update did not terminate and relaunch within 90 seconds");
            std::process::exit(1);
        }
        while let Ok(status) = events.try_recv() {
            println!("{status:?}");
            assert!(
                canceled_at.get().is_none(),
                "managed update restarted: {status:?}"
            );
            if status == Status::Ready {
                if std::env::var_os("CGX_SMOKE_MANAGED").is_some() {
                    backend.set_managed(true);
                    backend.install();
                    assert_eq!(events.try_recv().ok(), Some(Status::Idle));
                    assert!(events.try_recv().is_err());
                    println!("managed policy canceled the staged install");
                    canceled_at.set(Some(Instant::now()));
                } else {
                    backend.install();
                }
            }
        }
    });
    let _timer: Retained<AnyObject> = unsafe {
        msg_send![class!(NSTimer), scheduledTimerWithTimeInterval: 0.01_f64, repeats: true, block: &*poll]
    };
    unsafe {
        let _: () = msg_send![app, run];
    }
    Ok(())
}

#[cfg(not(target_os = "macos"))]
fn main() {}
