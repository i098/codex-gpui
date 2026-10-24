#!/usr/bin/env python3
import json
from pathlib import Path
import subprocess
import sys


def notices(metadata):
    packages = {package["id"]: package for package in metadata["packages"]}
    nodes = {node["id"]: node for node in metadata["resolve"]["nodes"]}
    roots = [package["id"] for package in metadata["packages"] if package["name"] == "app"]
    if len(roots) != 1:
        raise ValueError("expected one app package")
    selected = set()
    pending = roots.copy()
    while pending:
        package_id = pending.pop()
        if package_id in selected:
            continue
        selected.add(package_id)
        pending.extend(dependency["pkg"] for dependency in nodes[package_id]["deps"]
                       if any(kind["kind"] != "dev" for kind in dependency["dep_kinds"]))
    workspace = Path(metadata["workspace_root"])
    output = ["Third-party notices for Codex GPUI\n"]
    for package in sorted((packages[key] for key in selected), key=lambda item: (item["name"], item["version"], item["id"])):
        directory = Path(package["manifest_path"]).parent
        if directory.parent == workspace / "crates":
            continue
        output.append(f"\n{'=' * 72}\n{package['name']} {package['version']}\n")
        output.append(f"License: {package.get('license') or 'See license file'}\n")
        if package.get("repository"):
            output.append(f"Source: {package['repository']}\n")
        paths = set()
        if package.get("license_file"):
            declared = directory / package["license_file"]
            if not declared.is_file():
                raise FileNotFoundError(declared)
            paths.add(declared)
        for pattern in ("LICENSE*", "LICENCE*", "COPYING*", "NOTICE*", "COPYRIGHT*", "UNLICENSE*"):
            paths.update(path for path in directory.glob(pattern) if path.is_file())
        if not paths and not package.get("license"):
            raise ValueError(f"no license metadata or license file for {package['name']}")
        for path in sorted(paths):
            output.append(f"\n--- {path.name} ---\n")
            output.append(path.read_text(encoding="utf-8"))
            output.append("\n")
    return "".join(output)


def main():
    if len(sys.argv) != 2:
        raise SystemExit("usage: third-party-notices.py OUTPUT")
    metadata = json.loads(subprocess.check_output([
        "cargo", "metadata", "--locked", "--format-version", "1",
        "--filter-platform", "aarch64-apple-darwin",
    ], text=True))
    output = Path(sys.argv[1])
    output.write_text(notices(metadata), encoding="utf-8")


if __name__ == "__main__":
    main()
