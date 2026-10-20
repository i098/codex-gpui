#!/usr/bin/env python3
"""Capture an original/native census cell on the Mac, then retain its strict diff."""

import argparse
import concurrent.futures
import hashlib
import json
import os
from pathlib import Path
import shlex
import signal
import socket
import subprocess
import sys
import time
import urllib.request

ROOT = Path(__file__).resolve().parents[2]
CELLS = {("home", "rest"), ("terminal", "rest"), ("settings", "configuration")}
PINS = {
    "index.html": "fbdbdbc3d10bc13ebb49e6175673c77fafbcbcdec90a7f7e147ad91950145a0b",
    "assets/index-44a69cb8b3cd.js": "bd10e3c9056f5dc7f6a9ce750d6cfb6a99343e20e9a206bef6d61d4b70df6dea",
    "assets/app-initial-6350d04672d0.js": "604b5d2de4fbb41f3d0dc5b3662129c558e1c7b8aac8cc87225286f58dcfaf8f",
    "assets/app-shared-8bd9d0399038.js": "607ad0866dd25ed3764711b4467a9b60708353a599485b5249ff4cbe0e870245",
    "assets/app-shared-9a94b907ed47.css": "69f929f71947824fed18365d54c8aa0c975a856649339e585a50f6f547e215d7",
    "assets/app-initial-99e8b4b455fa.css": "1a75f9180b89d2457dc09e4efea288436c9cf6d44e5ba59b2c03d3d571b7bf61",
    "assets/app-primary-6a01c6b721a4.css": "4fa4b28e72c437f924e45aeeff2b4c26f23f37fe29048258779f816c212f1574",
}


def run(args, **kwargs):
    return subprocess.run(args, check=True, text=True, **kwargs)


def arguments():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("surface", choices=sorted({s for s, _ in CELLS}))
    parser.add_argument("state")
    parser.add_argument("--theme", choices=["dark", "light"], default="dark")
    parser.add_argument("--size", default="1280x832")
    parser.add_argument("--out", required=True, type=Path)
    parser.add_argument("--mac", default="mac")
    parser.add_argument("--warm-target", required=True, help="explicitly handed-off Mac Cargo target")
    parser.add_argument("--webview", type=Path, default=Path("/tmp/cgx/asar-x/webview"))
    parser.add_argument("--cdp", default="http://127.0.0.1:9467")
    parser.add_argument("--worker", action="store_true", help=argparse.SUPPRESS)
    args = parser.parse_args()
    if (args.surface, args.state) not in CELLS:
        parser.error("no original/native adapter for this cell")
    try:
        args.width, args.height = (int(n) for n in args.size.split("x"))
        if (args.width, args.height) not in {(1280, 832), (1440, 900), (767, 832), (600, 832)}:
            raise ValueError()
    except ValueError:
        parser.error("size must be 1280x832, 1440x900, 767x832, or 600x832")
    if args.width < 1000 and args.surface != "home":
        parser.error("only Home has a narrow-size navigation adapter")
    return args


def pin_assets(webview):
    for name, expected in PINS.items():
        actual = hashlib.sha256((webview / name).read_bytes()).hexdigest()
        if actual != expected:
            raise RuntimeError(f"original asset pin mismatch: {name}: {actual}")


def build(env, output):
    with (output / "build.log").open("w") as log:
        result = run(["cargo", "test", "--locked", "--offline", "-p", "app", "--test",
                      "proof_matrix", "--no-run", "--message-format=json"],
                     cwd=ROOT, env=env, stdout=subprocess.PIPE, stderr=log, timeout=900)
    for line in result.stdout.splitlines():
        message = json.loads(line)
        if message.get("executable") and message.get("target", {}).get("name") == "proof_matrix":
            return message["executable"]
    raise RuntimeError("Cargo returned no proof_matrix executable")


def capture_original(args, env, output):
    with (output / "original.log").open("w") as log:
        run(["node", "tools/parity/original.mjs", args.surface, args.state,
             args.theme, str(args.width), str(args.height), str(output / "refs")],
            cwd=ROOT, env=env, stdout=log, stderr=subprocess.STDOUT, timeout=240)


def worker(args):
    if sys.platform != "darwin":
        raise RuntimeError("the capture worker requires macOS")
    output = args.out.resolve()
    output.mkdir(parents=True, exist_ok=True)
    pin_assets(args.webview)
    with urllib.request.urlopen(args.cdp + "/json/version", timeout=10) as response:
        browser = json.load(response)
    if browser["Browser"] != "HeadlessChrome/155.0.8059.79" or "a49e2e3d4addfd84e765b52afe230bedb81096db" not in browser["WebKit-Version"]:
        raise RuntimeError(f"unpinned browser: {browser}")
    scratch = ROOT.parent
    env = dict(os.environ)
    env.pop("R1_SF_FONTS", None)
    toolchain = "/tmp/cgx-mac/rustup/toolchains/1.98.1-aarch64-apple-darwin/bin"
    env.update(PATH=f"{scratch}/bin:{toolchain}:/opt/homebrew/bin:/usr/bin:/bin:/usr/sbin:/sbin",
               CARGO_HOME="/tmp/cgx-mac/cargo", CARGO_INCREMENTAL="0",
               CARGO_TARGET_DIR=str(scratch / "target"), TMPDIR=str(scratch / "tmp"),
               R1_OFFLINE="1", R1_WEBVIEW=str(args.webview), R1_CDP=args.cdp,
               MOCK_FIXTURES=str(ROOT / "fixtures"), MOCK_LOG=str(output / "requests.log"))
    (scratch / "tmp").mkdir()
    (scratch / "bin").mkdir()
    shim = scratch / "bin/ssh"
    shim.write_text("#!/bin/sh\nexec /opt/homebrew/bin/node " + shlex.quote(str(ROOT / "tools/route1/linux/mock-app-server.mjs")) + "\n")
    shim.chmod(0o700)
    run(["cp", "-cR", args.warm_target, str(scratch / "target")])
    for source in ROOT.rglob("*"):
        if source.is_file() and (source.suffix == ".rs" or source.name == "Cargo.toml"):
            source.touch()
    with socket.socket() as sock:
        sock.bind(("127.0.0.1", 0))
        port = sock.getsockname()[1]
    env.update(R1_PORT=str(port), R1_SERVER=f"http://127.0.0.1:{port}")
    (output / "provenance.json").write_text(json.dumps({
        "surface": args.surface, "state": args.state, "theme": args.theme,
        "size": args.size, "browser": browser, "asset_sha256": PINS,
        "fonts": "macOS native; R1_SF_FONTS unset", "warm_target": args.warm_target,
    }, indent=2) + "\n")
    with (output / "server.log").open("w") as log:
        server = subprocess.Popen(["node", "tools/parity/fixture-server.mjs"], cwd=ROOT, env=env,
                                  stdout=log, stderr=subprocess.STDOUT, start_new_session=True)
        try:
            for _ in range(100):
                if server.poll() is not None:
                    raise RuntimeError("fixture server exited; see server.log")
                try:
                    with urllib.request.urlopen(env["R1_SERVER"] + "/index.html", timeout=1):
                        break
                except OSError:
                    time.sleep(0.1)
            else:
                raise RuntimeError("fixture server did not start")
            with concurrent.futures.ThreadPoolExecutor(max_workers=2) as pool:
                executable = pool.submit(build, env, output)
                original = pool.submit(capture_original, args, env, output)
                original.result()
                binary = executable.result()
            env.update(CGX_PROOF_SURFACE=args.surface, CGX_PROOF_STATE=args.state,
                       CGX_PROOF_THEME=args.theme, CGX_PROOF_WIDTH=str(args.width),
                       CGX_PROOF_HEIGHT=str(args.height), CGX_PROOF_OUT=str(output))
            with (output / "native.log").open("w") as native_log:
                result = subprocess.run([binary, "--ignored", "--exact", "capture_cell", "--nocapture"],
                                        cwd=ROOT, env=env, stdout=native_log,
                                        stderr=subprocess.STDOUT, timeout=180)
            return result.returncode
        finally:
            os.killpg(server.pid, signal.SIGTERM)
            try:
                server.wait(timeout=10)
            except subprocess.TimeoutExpired:
                os.killpg(server.pid, signal.SIGKILL)
                server.wait()


def dispatch(args):
    args.out = args.out.resolve()
    if args.out.exists():
        raise RuntimeError(f"output already exists: {args.out}")
    pin_assets(args.webview)
    head = run(["git", "rev-parse", "HEAD"], cwd=ROOT, stdout=subprocess.PIPE).stdout.strip()
    ssh = ["ssh", "-o", "BatchMode=yes", "-o", "ConnectTimeout=15",
           "-o", "ControlMaster=no", "-o", "ControlPath=none", args.mac]
    transfer = ["rsync", "-a", "-e", shlex.join(ssh[:-1])]
    scratch = run(ssh + ["mktemp -d /tmp/cgx-proof.XXXXXXXX"], stdout=subprocess.PIPE).stdout.strip()
    if not scratch.startswith("/tmp/cgx-proof.") or not scratch.replace("/tmp/cgx-proof.", "").isalnum():
        raise RuntimeError(f"unexpected remote scratch path: {scratch!r}")
    args.out.mkdir(parents=True)
    retrieved = False
    try:
        run(transfer + ["--exclude=.git", "--exclude=target", str(ROOT) + "/", f"{args.mac}:{scratch}/repo/"])
        run(transfer + [str(args.webview) + "/", f"{args.mac}:{scratch}/webview/"])
        command = ["/usr/bin/python3", f"{scratch}/repo/tools/parity/proof.py", args.surface,
                   args.state, "--worker", "--theme", args.theme, "--size", args.size,
                   "--out", f"{scratch}/output", "--warm-target", args.warm_target,
                   "--webview", f"{scratch}/webview", "--cdp", args.cdp]
        with (args.out / "worker.log").open("w") as log:
            result = subprocess.run(ssh + [shlex.join(command)], stdout=log, stderr=subprocess.STDOUT, timeout=1800)
        run(transfer + [f"{args.mac}:{scratch}/output/", str(args.out) + "/"])
        retrieved = True
        (args.out / "source-head.txt").write_text(head + "\n")
        print(f"{'PASS' if result.returncode == 0 else 'FAIL'} {args.out}/differences.md")
        return result.returncode
    finally:
        if retrieved:
            run(ssh + ["rm -rf -- " + shlex.quote(scratch)])
        else:
            print(f"Retrieval did not complete; retained Mac scratch: {args.mac}:{scratch}", file=sys.stderr)


if __name__ == "__main__":
    args = arguments()
    sys.exit(worker(args) if args.worker else dispatch(args))
