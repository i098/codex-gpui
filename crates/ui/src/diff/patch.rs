use std::borrow::Cow;

fn unquote(path: &str) -> Option<Cow<'_, str>> {
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

fn without_prefix(path: Cow<'_, str>, bytes: usize) -> Cow<'_, str> {
    match path {
        Cow::Borrowed(path) => Cow::Borrowed(&path[bytes..]),
        Cow::Owned(mut path) => {
            path.replace_range(..bytes, "");
            Cow::Owned(path)
        }
    }
}

fn prefix_length(before: &str, after: &str) -> Option<usize> {
    if before == after {
        return Some(0);
    }
    let (old_prefix, old_path) = before.split_once('/')?;
    let (new_prefix, new_path) = after.split_once('/')?;
    (old_prefix != new_prefix && old_path == new_path).then_some(new_prefix.len() + 1)
}

fn header_path(header: &str) -> Option<Cow<'_, str>> {
    if header.starts_with('"') {
        let mut escaped = false;
        for (at, ch) in header.char_indices().skip(1) {
            if ch == '"' && !escaped {
                let before = unquote(&header[..=at])?;
                let after = unquote(header.get(at + 1..)?.strip_prefix(' ')?)?;
                let bytes = prefix_length(&before, &after)?;
                return Some(without_prefix(after, bytes));
            }
            escaped = ch == '\\' && !escaped;
        }
        return None;
    }
    if let Some((before, after)) = header.split_at_checked(header.len() / 2)
        && after.strip_prefix(' ') == Some(before)
    {
        return Some(Cow::Borrowed(before));
    }
    for (at, _) in header.match_indices(' ') {
        let before = &header[..at];
        let after = &header[at + 1..];
        if let Some(bytes) = prefix_length(before, after) {
            return Some(Cow::Borrowed(&after[bytes..]));
        }
    }
    None
}

/// Returns the target path from one Git file patch, or the old path for a deletion.
/// Quoted names are decoded as UTF-8; malformed escapes or missing headers return `None`.
/// Complete diff headers preserve no-prefix and mnemonic-prefix filenames.
/// Unquoted names are borrowed. This function does not resolve or authorize filesystem paths.
///
/// ```
/// use ui::screens::diff::panel::patch::file_path;
/// assert_eq!(file_path("+++ b/src/lib.rs\n").as_deref(), Some("src/lib.rs"));
/// ```
pub fn file_path(patch: &str) -> Option<Cow<'_, str>> {
    let headers = patch.lines().take_while(|line| {
        !line.starts_with("@@ ")
            && !line.starts_with("Binary files ")
            && *line != "GIT binary patch"
    });
    for prefix in ["rename to ", "copy to "] {
        if let Some(path) = headers.clone().find_map(|line| line.strip_prefix(prefix)) {
            return unquote(path);
        }
    }
    if let Some(path) = headers
        .clone()
        .find_map(|line| line.strip_prefix("diff --git ").and_then(header_path))
    {
        return Some(path);
    }
    for prefix in ["+++ ", "--- ", "rename from ", "copy from "] {
        if let Some(path) = headers.clone().find_map(|line| line.strip_prefix(prefix)) {
            let file_header = matches!(prefix, "+++ " | "--- ");
            let path = if file_header {
                path.split_once('\t').map_or(path, |(path, _)| path)
            } else {
                path
            };
            let path = unquote(path)?;
            if path == "/dev/null" {
                continue;
            }
            let bytes =
                usize::from(file_header && (path.starts_with("a/") || path.starts_with("b/"))) * 2;
            return Some(without_prefix(path, bytes));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quoted_names_decode_git_escapes_without_trimming_real_spaces() {
        assert_eq!(
            file_path("+++ \"b/quote\\\"slash\\\\line\\n\\303\\251.txt\"\n").as_deref(),
            Some("quote\"slash\\line\né.txt"),
        );
        assert!(matches!(
            file_path("+++ b/ space name.txt \t\n"),
            Some(Cow::Borrowed(" space name.txt ")),
        ));
        for invalid in [
            "+++ \"b/bad\\q\"\n",
            "+++ \"b/\\777\"\n",
            "+++ \"b/\\377\"\n",
        ] {
            assert!(file_path(invalid).is_none());
        }
    }

    #[test]
    fn metadata_only_changes_keep_their_target_path() {
        for (patch, expected) in [
            (
                "diff --git a/binary.bin b/binary.bin\nBinary files a/binary.bin and b/binary.bin differ\n",
                "binary.bin",
            ),
            (
                "diff --git a/old name b/new name\nsimilarity index 100%\nrename from old name\nrename to new name\n",
                "new name",
            ),
            (
                "diff --git a/empty.txt b/empty.txt\nnew file mode 100644\nindex 0000000..e69de29\n",
                "empty.txt",
            ),
            (
                "--- a/deleted name.txt\t\n+++ /dev/null\n",
                "deleted name.txt",
            ),
            (
                "diff --git \"a/caf\\303\\251.txt\" \"b/caf\\303\\251.txt\"\nGIT binary patch\n",
                "café.txt",
            ),
            (
                "diff --git a/copy-source.txt b/copy-target.txt\nsimilarity index 100%\ncopy from copy-source.txt\ncopy to copy-target.txt\n",
                "copy-target.txt",
            ),
        ] {
            assert_eq!(file_path(patch).as_deref(), Some(expected));
        }
    }

    #[test]
    fn git_prefix_settings_do_not_change_the_real_filename() {
        for patch in [
            "diff --git a/file.txt a/file.txt\n--- a/file.txt\n+++ a/file.txt\n@@ -1 +1 @@\n-old\n+new\n",
            "diff --git i/a/file.txt w/a/file.txt\n--- i/a/file.txt\n+++ w/a/file.txt\n@@ -1 +1 @@\n-old\n+new\n",
            "diff --git a/a/file.txt b/a/file.txt\nBinary files a/a/file.txt and b/a/file.txt differ\n",
        ] {
            assert_eq!(file_path(patch).as_deref(), Some("a/file.txt"));
        }
    }

    #[test]
    fn added_code_cannot_replace_file_metadata() {
        assert_eq!(
            file_path("diff --git a/safe.txt b/safe.txt\n@@ -1 +1 @@\n+++ b/wrong.txt\n")
                .as_deref(),
            Some("safe.txt"),
        );
        assert!(file_path("@@ -1 +1 @@\n+++ b/wrong.txt\n").is_none());
    }
}
