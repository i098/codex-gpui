use anyhow::{Context as _, Result};
use std::{
    collections::HashSet,
    fs,
    io::ErrorKind,
    path::{Path, PathBuf},
};

pub(super) fn discover(home: &Path) -> Result<Vec<String>> {
    let root = home.join(".ssh");
    let mut visited = HashSet::new();
    let mut aliases = HashSet::new();
    read_config(
        &root.join("config"),
        &root,
        home,
        &mut visited,
        &mut aliases,
    )?;
    let mut aliases: Vec<_> = aliases
        .into_iter()
        .filter(|alias| !alias.eq_ignore_ascii_case("colima"))
        .collect();
    aliases.sort();
    Ok(aliases)
}

fn host_words(value: &str) -> Result<Vec<String>> {
    let mut words = Vec::new();
    let mut word = String::new();
    let mut quoted = false;
    let mut escaped = false;
    for character in value.chars() {
        if escaped {
            word.extend((character != '"').then_some('\\'));
            word.push(character);
            escaped = false;
            continue;
        }
        match character {
            '\\' => escaped = true,
            '"' => quoted = !quoted,
            '#' if !quoted && !words.is_empty() => break,
            ' ' | '\t' | '=' if !quoted => finish_word(&mut word, &mut words),
            _ => word.push(character),
        }
    }
    anyhow::ensure!(!escaped && !quoted, "Invalid SSH config quoting");
    finish_word(&mut word, &mut words);
    anyhow::ensure!(!words.is_empty(), "Missing SSH host value");
    Ok(words)
}

fn finish_word(word: &mut String, words: &mut Vec<String>) {
    if !word.is_empty() {
        words.push(std::mem::take(word));
    }
}

fn host_alias(value: &str) -> Result<Option<String>> {
    let words = host_words(value)?;
    let literal = |value: &&str| !value.contains(['!', '*', '?', '[', ']']);
    let alias = if words.len() == 1 {
        words[0].split_whitespace().find(literal)
    } else {
        words.iter().map(String::as_str).find(literal)
    };
    Ok(alias.map(str::to_owned))
}

fn read_config(
    path: &Path,
    root: &Path,
    home: &Path,
    visited: &mut HashSet<PathBuf>,
    aliases: &mut HashSet<String>,
) -> Result<()> {
    let source = match fs::read_to_string(path) {
        Ok(source) => source,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error).context("read SSH config"),
    };
    if !visited.insert(path.canonicalize()?) {
        return Ok(());
    }
    let mut top_level = true;
    for line in source.lines().map(str::trim_start) {
        if line.starts_with('#') {
            continue;
        }
        let Some((name, value)) =
            line.split_once(|character: char| character.is_whitespace() || character == '=')
        else {
            continue;
        };
        let value = value.trim_start().trim_start_matches('=').trim();
        if name.eq_ignore_ascii_case("Host") {
            top_level = false;
            if let Some(alias) = host_alias(value)? {
                aliases.insert(alias);
            }
        } else if name.eq_ignore_ascii_case("Match") {
            top_level = false;
        } else if top_level && name.eq_ignore_ascii_case("Include") {
            read_includes(value, root, home, visited, aliases)?;
        }
    }
    Ok(())
}

fn read_includes(
    value: &str,
    root: &Path,
    home: &Path,
    visited: &mut HashSet<PathBuf>,
    aliases: &mut HashSet<String>,
) -> Result<()> {
    if value.is_empty() || value.starts_with('#') {
        return Ok(());
    }
    let patterns = host_words(value)?;
    for pattern in patterns
        .iter()
        .flat_map(|pattern| pattern.split_whitespace())
    {
        for entry in include_paths(pattern, root, home)? {
            let path = entry?;
            if path.metadata()?.is_file() {
                read_config(&path, root, home, visited, aliases)?;
            }
        }
    }
    Ok(())
}

fn include_paths(pattern: &str, root: &Path, home: &Path) -> Result<glob::Paths> {
    let path = if pattern == "~" {
        home.to_path_buf()
    } else if let Some(rest) = pattern
        .strip_prefix("~/")
        .or_else(|| pattern.strip_prefix("~\\"))
    {
        home.join(rest)
    } else {
        root.join(pattern)
    };
    Ok(glob::glob_with(
        path.to_str().context("SSH include path is not UTF-8")?,
        glob::MatchOptions {
            require_literal_leading_dot: true,
            ..glob::MatchOptions::new()
        },
    )?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_quotes_follow_the_original_parser() -> Result<()> {
        assert_eq!(
            host_alias("\"build host\" other")?.as_deref(),
            Some("build host")
        );
        assert_eq!(host_alias("'build host' other")?.as_deref(), Some("'build"));
        assert_eq!(host_alias("\"build host\"")?.as_deref(), Some("build"));
        assert!(host_alias("\"broken").is_err());
        Ok(())
    }

    #[test]
    fn includes_keep_one_literal_alias_per_host_and_break_cycles() -> Result<()> {
        let home = tempfile::tempdir()?;
        let ssh = home.path().join(".ssh");
        fs::create_dir(&ssh)?;
        fs::write(
            ssh.join("config"),
            "# Ignored 'quote\nInclude = more-* *.conf ~\nHost * !nope \"build host\" other\nHost COLIMA\nInclude ignored\n",
        )?;
        fs::write(
            ssh.join("more-hosts"),
            "Include config\nHost = deploy\nHost github.com\nHost deploy\n",
        )?;
        fs::write(ssh.join("ignored"), "Host must-not-appear\n")?;
        fs::write(ssh.join(".ignored.conf"), [0xff])?;
        fs::write(ssh.join("~"), "Host must-not-appear\n")?;
        assert_eq!(
            discover(home.path())?,
            ["build host", "deploy", "github.com"]
        );
        Ok(())
    }
}
