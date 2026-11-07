use std::{env, fs, path::PathBuf, process::Command};

const SKIA_TOML: &str = include_str!("../../skia.toml");

/// Value of `key = "value"` in skia.toml.
fn skia_fact(key: &str) -> &'static str {
    SKIA_TOML
        .lines()
        .filter_map(|line| line.split_once('='))
        .find(|(name, _)| name.trim() == key)
        .map(|(_, value)| value.trim().trim_matches('"'))
        .unwrap_or_else(|| panic!("skia.toml has no `{key}`"))
}

fn output(command: &mut Command) -> String {
    let result = command.output().expect("run package tool");
    assert!(
        result.status.success(),
        "package tool failed: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    String::from_utf8(result.stdout).expect("package tool output is UTF-8")
}

fn main() {
    println!("cargo:rerun-if-changed=../../skia.toml");
    let version = skia_fact("version");
    let skia_revision = skia_fact("skia_revision");
    let skia_archive_sha256 = skia_fact("skia_archive_sha256");
    println!("cargo:rerun-if-env-changed=SKIA_HAIRLINE_BINARIES_URL");
    println!("cargo:rerun-if-env-changed=SKIA_HAIRLINE_PACKAGE_SHA256");
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("macos") {
        return;
    }
    assert_eq!(
        env::var("CARGO_CFG_TARGET_ARCH").as_deref(),
        Ok("aarch64"),
        "the Skia hairline package supports Apple silicon only"
    );

    let url = env::var("SKIA_HAIRLINE_BINARIES_URL").expect("set the versioned Skia package URL");
    let expected_hash =
        env::var("SKIA_HAIRLINE_PACKAGE_SHA256").expect("set the versioned Skia package SHA-256");
    let out = PathBuf::from(env::var_os("OUT_DIR").expect("Cargo sets OUT_DIR"));
    let archive = out.join(format!("skia-hairline-{version}.tar.gz"));
    let mut curl = Command::new("curl");
    output(
        curl.args([
            "--fail",
            "--silent",
            "--show-error",
            "--location",
            "--output",
        ])
        .arg(&archive)
        .arg(url),
    );
    let mut openssl = Command::new("openssl");
    let digest = output(openssl.args(["dgst", "-sha256", "-r"]).arg(&archive));
    let actual_hash = digest.split_whitespace().next().expect("SHA-256 output");
    assert_eq!(
        actual_hash, expected_hash,
        "versioned Skia package hash mismatch"
    );

    let listing = output(Command::new("tar").arg("-tzf").arg(&archive));
    let mut files: Vec<_> = listing.lines().collect();
    files.sort_unstable();
    assert_eq!(files, ["libskia_hairline.a", "manifest.json"]);
    output(
        Command::new("tar")
            .arg("-xzf")
            .arg(&archive)
            .arg("-C")
            .arg(&out),
    );

    let manifest = fs::read_to_string(out.join("manifest.json")).expect("package manifest");
    for identity in [
        "\"package\":\"skia-hairline-sys\"".into(),
        format!("\"version\":\"{version}\""),
        "\"target\":\"aarch64-apple-darwin\"".into(),
        format!("\"skia_revision\":\"{skia_revision}\""),
        format!("\"skia_archive_sha256\":\"{skia_archive_sha256}\""),
    ] {
        assert!(
            manifest.contains(&identity),
            "package identity mismatch: {identity}"
        );
    }

    println!("cargo:rustc-link-search=native={}", out.display());
    println!("cargo:rustc-link-lib=static=skia_hairline");
}
