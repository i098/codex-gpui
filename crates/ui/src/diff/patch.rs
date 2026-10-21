use std::borrow::Cow;

pub(super) fn unquote(path: &str) -> Option<Cow<'_, str>> {
    if !path.starts_with('"') {
        return Some(Cow::Borrowed(path));
    }
    let bytes = path.strip_prefix('"')?.strip_suffix('"')?.as_bytes();
    let mut result = Vec::with_capacity(bytes.len());
    let mut at = 0;
    while at < bytes.len() {
        let byte = bytes[at];
        at += 1;
        if byte != b'\\' {
            result.push(byte);
            continue;
        }
        let escaped = *bytes.get(at)?;
        at += 1;
        result.push(match escaped {
            b'a' => 7,
            b'b' => 8,
            b't' => b'\t',
            b'n' => b'\n',
            b'v' => 11,
            b'f' => 12,
            b'r' => b'\r',
            b'\\' | b'"' => escaped,
            b'0'..=b'7' => {
                let mut value = u16::from(escaped - b'0');
                for _ in 0..2 {
                    let next = *bytes.get(at)?;
                    if !(b'0'..=b'7').contains(&next) {
                        return None;
                    }
                    value = value * 8 + u16::from(next - b'0');
                    at += 1;
                }
                u8::try_from(value).ok()?
            }
            _ => return None,
        });
    }
    String::from_utf8(result).ok().map(Cow::Owned)
}

pub(super) fn path(patch: &str) -> Option<Cow<'_, str>> {
    for prefix in ["+++ ", "rename to ", "--- ", "rename from "] {
        if let Some(path) = patch.lines().find_map(|line| line.strip_prefix(prefix)) {
            let path = unquote(path)?;
            if path == "/dev/null" {
                continue;
            }
            let strip_side = matches!(prefix, "+++ " | "--- ");
            return Some(match path {
                Cow::Borrowed(path) => Cow::Borrowed(if strip_side {
                    path.strip_prefix("a/")
                        .or_else(|| path.strip_prefix("b/"))
                        .unwrap_or(path)
                } else {
                    path
                }),
                Cow::Owned(path) => Cow::Owned(if strip_side {
                    path.strip_prefix("a/")
                        .or_else(|| path.strip_prefix("b/"))
                        .unwrap_or(&path)
                        .to_owned()
                } else {
                    path
                }),
            });
        }
    }
    let header = patch
        .lines()
        .find_map(|line| line.strip_prefix("diff --git "))?;
    if header.starts_with('"') {
        let mut escaped = false;
        for (at, ch) in header.char_indices().skip(1) {
            if ch == '"' && !escaped {
                let target = unquote(header.get(at + 2..)?)?;
                return Some(Cow::Owned(
                    target.strip_prefix("b/").unwrap_or(&target).to_owned(),
                ));
            }
            escaped = ch == '\\' && !escaped;
        }
    }
    for (at, _) in header.match_indices(" b/") {
        let left = header[..at].strip_prefix("a/")?;
        let right = &header[at + 3..];
        if left == right {
            return Some(Cow::Borrowed(right));
        }
    }
    None
}

pub(super) fn empty_state(patch: &str) -> &'static str {
    if patch
        .lines()
        .any(|line| line.starts_with("Binary files ") || line == "GIT binary patch")
    {
        "Binary file not shown"
    } else if patch.lines().any(|line| line.starts_with("rename from ")) {
        "File renamed without changes"
    } else {
        "No content"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::screens::diff::panel::controls::files;

    #[test]
    fn metadata_only_changes_keep_their_file_and_original_placeholder() {
        for (patch, name, label) in [
            (
                "diff --git a/picture.png b/picture.png\nBinary files a/picture.png and b/picture.png differ\n",
                "picture.png",
                "Binary file not shown",
            ),
            (
                "diff --git a/old.txt b/new.txt\nsimilarity index 100%\nrename from old.txt\nrename to new.txt\n",
                "new.txt",
                "File renamed without changes",
            ),
            (
                "diff --git a/empty.txt b/empty.txt\nnew file mode 100644\nindex 0000000..e69de29\n",
                "empty.txt",
                "No content",
            ),
        ] {
            let parsed = files(patch);
            assert_eq!(parsed.len(), 1);
            assert_eq!(parsed[0].name.as_ref(), name);
            assert_eq!(parsed[0].problem, Some(label));
            assert!(parsed[0].hunks.is_empty());
        }
    }

    #[test]
    fn git_quoted_paths_preserve_spaces_newlines_and_unicode() {
        assert_eq!(
            path("diff --git a/a b/file b/a b/file\n").as_deref(),
            Some("a b/file")
        );
        assert_eq!(
            path("+++ \"b/space\\n\\303\\251.txt\"\n").as_deref(),
            Some("space\né.txt")
        );
        assert_eq!(
            path("--- a/deleted.txt\n+++ /dev/null\n").as_deref(),
            Some("deleted.txt")
        );
    }
}
