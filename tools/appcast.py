#!/usr/bin/env python3
"""Write the Sparkle appcast and SHA-256 checksum for a release zip.

usage: appcast.py ZIP --tag vX.Y.Z --repo OWNER/REPO --sign-update PATH --out DIR
The Ed25519 private key comes from SPARKLE_ED_PRIVATE_KEY (never argv). The feed URL baked
into the bundle is https://github.com/OWNER/REPO/releases/latest/download/appcast.xml.
"""
import argparse
import email.utils
import hashlib
import os
import re
import subprocess
import sys
import tempfile
from xml.sax.saxutils import quoteattr

SEMVER = re.compile(r"v?(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)")


def parse_version(text):
    match = SEMVER.fullmatch(text)
    if not match:
        raise ValueError(f"not MAJOR.MINOR.PATCH: {text!r}")
    return tuple(int(part) for part in match.groups())


def sha256_of(path):
    digest = hashlib.sha256()
    with open(path, "rb") as file:
        for block in iter(lambda: file.read(1 << 20), b""):
            digest.update(block)
    return digest.hexdigest()


def verify_checksum(path, expected):
    actual = sha256_of(path)
    if actual != expected.strip().split()[0].lower():
        raise ValueError(f"checksum mismatch for {path}: expected {expected.split()[0]}, got {actual}")


def sign(sign_update, key, zip_path):
    """Run Sparkle's sign_update and return (edSignature, length), checking the length."""
    with tempfile.NamedTemporaryFile("w", delete=False) as key_file:
        os.chmod(key_file.name, 0o600)
        key_file.write(key)
    try:
        out = subprocess.run(
            [sign_update, "--ed-key-file", key_file.name, zip_path],
            check=True, capture_output=True, text=True,
        ).stdout
    finally:
        os.unlink(key_file.name)
    signature = re.search(r'sparkle:edSignature="([^"]+)"', out)
    length = re.search(r'length="([0-9]+)"', out)
    if not signature or not length:
        raise ValueError("sign_update printed no signature")
    if int(length.group(1)) != os.path.getsize(zip_path):
        raise ValueError("sign_update length differs from the zip size")
    return signature.group(1), int(length.group(1))


def appcast(version, repo, zip_name, signature, length, pub_date, minimum_system="13.0"):
    parse_version(version)
    url = f"https://github.com/{repo}/releases/download/v{version}/{zip_name}"
    notes = f"https://github.com/{repo}/releases/tag/v{version}"
    return f"""<?xml version="1.0" encoding="utf-8"?>
<rss version="2.0" xmlns:sparkle="http://www.andymatuschak.org/xml-namespaces/sparkle">
  <channel>
    <title>Codex GPUI</title>
    <item>
      <title>Version {version}</title>
      <pubDate>{pub_date}</pubDate>
      <sparkle:version>{version}</sparkle:version>
      <sparkle:shortVersionString>{version}</sparkle:shortVersionString>
      <sparkle:minimumSystemVersion>{minimum_system}</sparkle:minimumSystemVersion>
      <sparkle:releaseNotesLink>{notes}</sparkle:releaseNotesLink>
      <enclosure url={quoteattr(url)} length="{length}" type="application/octet-stream" sparkle:edSignature={quoteattr(signature)}/>
    </item>
  </channel>
</rss>
"""


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("zip")
    parser.add_argument("--tag", required=True)
    parser.add_argument("--repo", required=True)
    parser.add_argument("--sign-update", required=True)
    parser.add_argument("--out", required=True)
    args = parser.parse_args()
    version = args.tag.removeprefix("v")
    parse_version(version)
    key = os.environ.get("SPARKLE_ED_PRIVATE_KEY")
    if not key:
        sys.exit("SPARKLE_ED_PRIVATE_KEY is not set")
    signature, length = sign(args.sign_update, key, args.zip)
    name = os.path.basename(args.zip)
    pub_date = email.utils.formatdate(usegmt=True)
    os.makedirs(args.out, exist_ok=True)
    with open(os.path.join(args.out, "appcast.xml"), "w") as file:
        file.write(appcast(version, args.repo, name, signature, length, pub_date))
    with open(os.path.join(args.out, name + ".sha256"), "w") as file:
        file.write(f"{sha256_of(args.zip)}  {name}\n")


if __name__ == "__main__":
    main()
