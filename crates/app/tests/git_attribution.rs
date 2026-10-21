use anyhow::{Result, ensure};
use app::host::{Host, LocalHost, Request, json_body};
use serde_json::{Value, json};
use std::{fs, path::Path, process::Command};

fn git(root: &Path, args: &[&str]) -> Result<String> {
    let output = Command::new("git").current_dir(root).args(args).output()?;
    ensure!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(String::from_utf8(output.stdout)?.trim_end().to_owned())
}

#[test]
fn commits_preserve_opt_out_existing_trailers_and_custom_authors() -> Result<()> {
    #[cfg(target_os = "macos")]
    std::panic::set_hook(Box::new(|info| {
        eprintln!("{info}");
        std::process::exit(101);
    }));
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let host = LocalHost::new(root.join("home"));
    git(root, &["init", "-q", "-b", "main"])?;
    git(root, &["config", "user.name", "Attribution proof"])?;
    git(root, &["config", "user.email", "proof@example.invalid"])?;
    let cases = [
        (
            "Change",
            None,
            "Change\n\nCo-authored-by: Codex <noreply@openai.com>",
        ),
        ("No attribution", Some(Value::Null), "No attribution"),
        ("Blank attribution", Some(json!("  ")), "Blank attribution"),
        (
            "Custom attribution",
            Some(json!(" Team <team@example.invalid> ")),
            "Custom attribution\n\nCo-authored-by: Team <team@example.invalid>",
        ),
        (
            "Already attributed\n\nco-authored-by: CODEX <noreply@openai.com>",
            None,
            "Already attributed\n\nco-authored-by: CODEX <noreply@openai.com>",
        ),
        (
            "Signed\n\nSigned-off-by: Author <author@example.invalid>",
            None,
            "Signed\n\nSigned-off-by: Author <author@example.invalid>\nCo-authored-by: Codex <noreply@openai.com>",
        ),
        (
            "Fix: subject",
            None,
            "Fix: subject\nCo-authored-by: Codex <noreply@openai.com>",
        ),
    ];
    for (index, (message, attribution, expected)) in cases.into_iter().enumerate() {
        fs::write(root.join("tracked"), index.to_string())?;
        let mut params = json!({"cwd": root, "message": message, "includeUnstaged": true});
        if let Some(attribution) = attribution {
            params["commitAttribution"] = attribution;
        }
        let result = json_body(&host.fetch(&Request::post("git-commit", Some(params)))?)?;
        ensure!(result["status"] == "success", "{result}");
        assert_eq!(
            git(root, &["log", "-1", "--format=%B"])?,
            expected,
            "case {index}"
        );
    }
    let before = git(root, &["rev-parse", "HEAD"])?;
    fs::write(root.join("tracked"), "unchanged on invalid request")?;
    ensure!(
        host.fetch(&Request::post(
            "git-commit",
            Some(json!({
                "cwd": root, "message": "Invalid attribution", "includeUnstaged": true,
                "commitAttribution": false,
            }))
        ))
        .is_err()
    );
    ensure!(git(root, &["rev-parse", "HEAD"])? == before);
    ensure!(git(root, &["diff", "--cached", "--name-only"])?.is_empty());
    Ok(())
}
