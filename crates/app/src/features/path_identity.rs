//! The original's path identity `WM(a, b)` (`app-shared` `GM`/`RM`/`gxt`): two spellings of one
//! location are equal, including Windows drive, UNC and WSL forms. Used to match worktree roots
//! against pending-archive roots.

fn backslashes_to_slashes(path: &str) -> String {
    path.replace('\\', "/")
}

fn strip_ci<'a>(text: &'a str, prefix: &str) -> Option<&'a str> {
    text.get(..prefix.len())
        .filter(|head| head.eq_ignore_ascii_case(prefix))
        .map(|_| &text[prefix.len()..])
}

/// `[a-zA-Z]:[\\/]` at the start.
fn starts_with_drive_separator(text: &str) -> bool {
    let bytes = text.as_bytes();
    bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && matches!(bytes[2], b'\\' | b'/')
}

/// `UM`: `\\?\UNC\x` is `\\x`; `\\?\C:\x` is `C:\x`.
fn strip_verbatim(path: &str) -> String {
    if let Some(rest) = strip_ci(path, r"\\?\UNC\") {
        return format!(r"\\{rest}");
    }
    match path.strip_prefix(r"\\?\") {
        Some(rest) if starts_with_drive_separator(rest) => rest.to_owned(),
        _ => path.to_owned(),
    }
}

/// `pxt`: drops a `\\?\` or `\\.\` prefix, turning a leading `UNC\` into `\\`.
fn strip_device_prefix(path: &str) -> String {
    if path.starts_with(r"\\?\") || path.starts_with(r"\\.\") {
        let rest = &path[4..];
        return match strip_ci(rest, r"UNC\") {
            Some(rest) => format!(r"\\{rest}"),
            None => rest.to_owned(),
        };
    }
    path.to_owned()
}

/// `//wsl$/host/rest` or `//wsl.localhost/host/rest`: the host and the rest after it.
fn wsl_parts(path: &str) -> Option<(&str, &str)> {
    let rest = path.strip_prefix("//")?;
    let rest = strip_ci(rest, "wsl$").or_else(|| strip_ci(rest, "wsl.localhost"))?;
    let rest = rest.strip_prefix('/')?;
    let (host, tail) = rest.split_once('/').unwrap_or((rest, ""));
    (!host.is_empty()).then_some((host, tail))
}

/// `/mnt/<drive>` followed by `/` or the end.
fn is_mnt_drive(path: &str) -> bool {
    strip_ci(path, "/mnt/").is_some_and(|rest| {
        let bytes = rest.as_bytes();
        bytes.first().is_some_and(u8::is_ascii_alphabetic) && (bytes.len() == 1 || bytes[1] == b'/')
    })
}

/// `zM`: `C:\` or `C:/`.
fn is_drive_path(path: &str) -> bool {
    starts_with_drive_separator(path)
}

/// `BM`: `\\server\share` or `//server/share`.
fn is_unc_path(path: &str) -> bool {
    let unc = |separator: char| {
        let double = format!("{separator}{separator}");
        path.strip_prefix(&double).is_some_and(|rest| {
            rest.split_once(separator).is_some_and(|(server, share)| {
                !server.is_empty() && !share.is_empty() && !server.contains(separator)
            })
        })
    };
    unc('\\') || unc('/')
}

/// `RM`: the lowercase `/mnt/<drive>/...` form of a Windows or WSL path.
fn mnt_form(path: &str) -> String {
    let device = strip_ci(path, r"\\?\UNC\").is_some()
        || path
            .strip_prefix(r"\\?\")
            .is_some_and(starts_with_drive_separator);
    let lowered = backslashes_to_slashes(&if device {
        strip_device_prefix(path)
    } else {
        path.to_owned()
    })
    .to_lowercase();
    if let Some((_, tail)) = wsl_parts(&lowered) {
        return if tail.is_empty() {
            "/".into()
        } else {
            format!("/{tail}")
        };
    }
    let drive = lowered.strip_prefix('/').unwrap_or(&lowered);
    let bytes = drive.as_bytes();
    if bytes.len() >= 2 && bytes[0].is_ascii_lowercase() && bytes[1] == b':' {
        let letter = &drive[..1];
        return match drive[2..].strip_prefix('/') {
            Some(rest) if !rest.is_empty() => format!("/mnt/{letter}/{rest}"),
            Some(_) => format!("/mnt/{letter}"),
            None if drive.len() == 2 => format!("/mnt/{letter}"),
            None => lowered,
        };
    }
    lowered
}

/// `GM`: the canonical spelling.
fn canonical(path: &str) -> String {
    let slashed = backslashes_to_slashes(&strip_verbatim(path));
    let local = slashed
        .strip_prefix('/')
        .filter(|path| starts_with_drive_separator(path))
        .unwrap_or(&slashed);
    let spelled = if let Some((host, tail)) = wsl_parts(&slashed) {
        let tail = format!("/{tail}");
        if is_mnt_drive(&tail) {
            format!("win32:{}", mnt_form(&tail))
        } else {
            format!("wsl:{}:{tail}", host.to_lowercase())
        }
    } else if is_drive_path(local) || is_unc_path(local) {
        format!("win32:{}", mnt_form(local))
    } else {
        slashed.clone()
    };
    if spelled == "/" {
        spelled
    } else {
        spelled.trim_end_matches('/').to_owned()
    }
}

/// `gxt`: the Windows-comparable form and whether the spelling named a Windows path itself.
fn comparable(canonical: &str) -> Option<(String, bool)> {
    if let Some(path) = canonical.strip_prefix("win32:") {
        Some((path.to_owned(), true))
    } else if is_mnt_drive(canonical) {
        Some((mnt_form(canonical), false))
    } else {
        None
    }
}

/// `WM(a, b)`.
pub(super) fn same_path(a: &str, b: &str) -> bool {
    let (a, b) = (canonical(a), canonical(b));
    if a == b {
        return true;
    }
    match (comparable(&a), comparable(&b)) {
        (Some((a, explicit_a)), Some((b, explicit_b))) => (explicit_a || explicit_b) && a == b,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::same_path;

    #[test]
    fn spellings_of_one_location_match_and_others_do_not() {
        for (a, b, same) in [
            ("/Users/me/w/a", "/Users/me/w/a/", true),
            ("/Users/me/w/a///", "/Users/me/w/a", true),
            ("/", "/", true),
            ("/Users/me/w/a", "/users/me/w/a", false),
            ("/Users/me/w/a", "/Users/me/w/b", false),
            (r"C:\w\a", "/mnt/c/w/a", true),
            (r"c:\W\A", "C:/w/a/", true),
            (r"\\?\C:\w\a", "C:/w/a", true),
            ("/C:/w/a", r"C:\w\a", true),
            ("/mnt/c/w/a", "/mnt/C/w/a", false),
            ("/mnt/c/w/a", "/mnt/c/w/a/", true),
            ("//wsl$/Ubuntu/mnt/c/w/a", r"C:\w\a", true),
            (
                "//wsl.localhost/Ubuntu/home/me",
                "//WSL$/ubuntu/home/me/",
                true,
            ),
            ("//wsl$/Ubuntu/home/me", "//wsl$/Debian/home/me", false),
            (r"\\server\share\x", "//server/share/x/", true),
            (r"C:\w\a", r"D:\w\a", false),
        ] {
            assert_eq!(same_path(a, b), same, "{a} vs {b}");
        }
    }
}
