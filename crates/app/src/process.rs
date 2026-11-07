use anyhow::Result;
use std::{
    process::{Child, ExitStatus},
    time::{Duration, Instant},
};

/// Waits for `child`; past `timeout` it kills and reaps the child and returns `None`.
pub fn wait_timeout(
    child: &mut Child,
    timeout: Duration,
    poll: Duration,
) -> std::io::Result<Option<ExitStatus>> {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(status) = child.try_wait()? {
            return Ok(Some(status));
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Ok(None);
        }
        std::thread::sleep(poll);
    }
}

/// Runs `read` on the background executor, retrying up to `retries` times
/// after 1 s, 2 s, 4 s, ... while `retryable` accepts the error.
pub async fn retry<T: Send + 'static>(
    executor: &gpui::BackgroundExecutor,
    retries: u32,
    mut read: impl Fn() -> Result<T> + Send + 'static,
    retryable: impl Fn(&anyhow::Error) -> bool,
) -> Result<T> {
    let mut attempt = 0;
    loop {
        let (result, next) = executor
            .spawn(async move {
                let result = read();
                (result, read)
            })
            .await;
        read = next;
        if attempt == retries || !result.as_ref().is_err_and(&retryable) {
            return result;
        }
        executor.timer(Duration::from_secs(1 << attempt)).await;
        attempt += 1;
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::process::Command;

    #[test]
    fn returns_status_of_finished_child() {
        let mut child = Command::new("sh").args(["-c", "exit 3"]).spawn().unwrap();
        let status = wait_timeout(
            &mut child,
            Duration::from_secs(10),
            Duration::from_millis(5),
        )
        .unwrap()
        .unwrap();
        assert_eq!(status.code(), Some(3));
    }

    #[test]
    fn kills_child_after_timeout() {
        let mut child = Command::new("sleep").arg("30").spawn().unwrap();
        let started = Instant::now();
        let status = wait_timeout(
            &mut child,
            Duration::from_millis(50),
            Duration::from_millis(5),
        )
        .unwrap();
        assert!(status.is_none());
        assert!(started.elapsed() < Duration::from_secs(10));
        assert!(child.try_wait().unwrap().is_some(), "child was reaped");
    }
}
