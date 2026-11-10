//! Raises the open-file soft limit so the `codex app-server` child and its hooks inherit a usable one.

const TARGET: u64 = 10240;
const WARN_BELOW: u64 = 4096;

/// Raises `RLIMIT_NOFILE` soft to `min(hard, 10240)` and logs when the result is under 4096.
#[cfg(unix)]
pub fn raise() {
    let mut limit = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    // SAFETY: `limit` is a valid, writable rlimit.
    if unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, &mut limit) } != 0 {
        crate::logging::write(&format!(
            "fd limit: getrlimit failed: {}",
            std::io::Error::last_os_error()
        ));
        return;
    }
    let wanted = (limit.rlim_max as u64).min(TARGET);
    if (limit.rlim_cur as u64) < wanted {
        limit.rlim_cur = wanted as libc::rlim_t;
        // SAFETY: `limit` is a valid rlimit with soft <= hard.
        if unsafe { libc::setrlimit(libc::RLIMIT_NOFILE, &limit) } != 0 {
            crate::logging::write(&format!(
                "fd limit: setrlimit({wanted}) failed: {}",
                std::io::Error::last_os_error()
            ));
        }
    }
    let soft = current_soft().unwrap_or(0);
    if soft < WARN_BELOW {
        crate::logging::write(&format!(
            "fd limit: soft limit {soft} is below {WARN_BELOW}; hooks may fail with EMFILE"
        ));
    }
}

#[cfg(not(unix))]
pub fn raise() {}

#[cfg(unix)]
fn current_soft() -> Option<u64> {
    let mut limit = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    // SAFETY: `limit` is a valid, writable rlimit.
    (unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, &mut limit) } == 0)
        .then_some(limit.rlim_cur as u64)
}

#[cfg(all(test, unix))]
mod tests {
    #[test]
    fn raise_lifts_soft_limit_to_min_hard_target() {
        super::raise();
        let mut limit = libc::rlimit {
            rlim_cur: 0,
            rlim_max: 0,
        };
        assert_eq!(
            unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, &mut limit) },
            0
        );
        assert!(limit.rlim_cur as u64 >= (limit.rlim_max as u64).min(super::TARGET));
    }
}
