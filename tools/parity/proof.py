#!/usr/bin/env python3
"""Capture an original/native census cell on the Mac, then retain its strict diff."""

import argparse
import concurrent.futures
from contextlib import contextmanager
import hashlib
import json
import os
from pathlib import Path
import shlex
import shutil
import signal
import socket
import subprocess
import sys
import time
import tempfile
import urllib.request

ROOT = Path(__file__).resolve().parents[2]
CELLS = {("home", "rest"), ("terminal", "rest"), ("settings", "configuration"),
         ("mobile", "get-started"), ("mobile", "continuation"), ("automations", "empty"),
         ("diff", "rest")}
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


@contextmanager
def background(command, env, log):
    with log.open("w") as stream:
        process = subprocess.Popen(command, cwd=ROOT, env=env, stdout=stream,
                                   stderr=subprocess.STDOUT, start_new_session=True)
        try:
            yield process
        finally:
            if process.poll() is None:
                os.killpg(process.pid, signal.SIGTERM)
            try:
                process.wait(timeout=10)
            except subprocess.TimeoutExpired:
                os.killpg(process.pid, signal.SIGKILL)
                process.wait()


def free_port():
    with socket.socket() as sock:
        sock.bind(("127.0.0.1", 0))
        return sock.getsockname()[1]


def ready(process, url):
    for _ in range(300):
        if process.poll() is not None:
            raise RuntimeError(f"process exited before {url} was ready")
        try:
            with urllib.request.urlopen(url, timeout=1) as response:
                return response.read()
        except OSError:
            time.sleep(0.1)
    raise RuntimeError(f"process did not serve {url}")


def worker(args):
    if sys.platform != "darwin":
        raise RuntimeError("the capture worker requires macOS")
    output = args.out.resolve()
    output.mkdir(parents=True, exist_ok=True)
    scratch = ROOT.parent
    env = dict(os.environ)
    toolchain = "/tmp/cgx-mac/rustup/toolchains/1.98.1-aarch64-apple-darwin/bin"
    env.update(PATH=f"{toolchain}:/opt/homebrew/bin:/usr/bin:/bin:/usr/sbin:/sbin",
               CARGO_HOME="/tmp/cgx-mac/cargo", CARGO_INCREMENTAL="0",
               CARGO_TARGET_DIR=str(scratch / "target"), TMPDIR=str(scratch / "tmp"))
    (scratch / "tmp").mkdir()
    run(["cp", "-cR", args.warm_target, str(scratch / "target")])
    for source in ROOT.rglob("*"):
        if source.is_file() and (source.suffix == ".rs" or source.name == "Cargo.toml"):
            source.touch()
    binary = build(env, output)
    for _ in range(300):
        if (output / "original-ready").exists():
            break
        time.sleep(1)
    else:
        raise RuntimeError("the original capture did not return")
    if (output / "original-error.txt").exists():
        raise RuntimeError((output / "original-error.txt").read_text())
    env.update(CGX_PROOF_SURFACE=args.surface, CGX_PROOF_STATE=args.state,
               CGX_PROOF_THEME=args.theme, CGX_PROOF_WIDTH=str(args.width),
               CGX_PROOF_HEIGHT=str(args.height), CGX_PROOF_OUT=str(output))
    with (output / "native.log").open("w") as native_log:
        result = subprocess.run([binary, "--ignored", "--exact", "capture_cell", "--nocapture"],
                                cwd=ROOT, env=env, stdout=native_log,
                                stderr=subprocess.STDOUT, timeout=180)
    return result.returncode


def original_through_tunnel(args, ssh):
    env = dict(os.environ)
    env.pop("R1_SF_FONTS", None)
    server_port, cdp_port = free_port(), free_port()
    with tempfile.TemporaryDirectory(prefix="route1-", dir=args.out) as temporary:
        local = Path(temporary)
        shutil.copytree(ROOT / "tools/route1", local / "tools/route1")
        (local / "fixtures").symlink_to(ROOT / "fixtures", target_is_directory=True)
        (local / "refs").symlink_to(ROOT / "refs", target_is_directory=True)
        (local / "tools/capture").symlink_to(ROOT / "tools/capture", target_is_directory=True)
        (local / "bin").mkdir()
        shim = local / "bin/ssh"
        shim.write_text("#!/bin/sh\nexec node " + shlex.quote(str(ROOT / "tools/route1/linux/mock-app-server.mjs")) + "\n")
        shim.chmod(0o700)
        env.update(PATH=f"{local}/bin:{env['PATH']}", R1_OFFLINE="1",
                   R1_WEBVIEW=str(args.webview.resolve()), R1_PORT=str(server_port),
                   R1_SERVER=f"http://127.0.0.1:{server_port}", R1_CDP=f"http://127.0.0.1:{cdp_port}",
                   MOCK_FIXTURES=str(ROOT / "fixtures"), MOCK_LOG=str(args.out / "requests.log"))
        tunnel_command = ssh[:-1] + [
            "-o", "ExitOnForwardFailure=yes", "-N",
            "-L", f"{cdp_port}:127.0.0.1:9467",
            "-R", f"{server_port}:127.0.0.1:{server_port}", args.mac,
        ]
        with background(["node", str(local / "tools/route1/server.mjs")], env, args.out / "server.log") as server, \
                background(tunnel_command, os.environ, args.out / "tunnel.log") as tunnel:
            ready(server, env["R1_SERVER"] + "/index.html")
            browser = json.loads(ready(tunnel, env["R1_CDP"] + "/json/version"))
            if browser["Browser"] != "HeadlessChrome/155.0.8059.79" or "a49e2e3d4addfd84e765b52afe230bedb81096db" not in browser["WebKit-Version"]:
                raise RuntimeError(f"unpinned browser: {browser}")
            (args.out / "provenance.json").write_text(json.dumps({
                "surface": args.surface, "state": args.state, "theme": args.theme,
                "size": args.size, "browser": browser, "asset_sha256": PINS,
                "fonts": "macOS native; R1_SF_FONTS unset", "warm_target": args.warm_target,
                "browser_ownership": "fresh disposeOnDetach context on the shared Mac browser",
            }, indent=2) + "\n")
            capture_original(args, env, args.out)


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
        command = ["/usr/bin/python3", f"{scratch}/repo/tools/parity/proof.py", args.surface,
                   args.state, "--worker", "--theme", args.theme, "--size", args.size,
                   "--out", f"{scratch}/output", "--warm-target", args.warm_target]
        def native_run():
            with (args.out / "worker.log").open("w") as log:
                return subprocess.run(ssh + [shlex.join(command)], stdout=log, stderr=subprocess.STDOUT, timeout=1800)
        with concurrent.futures.ThreadPoolExecutor(max_workers=1) as pool:
            native = pool.submit(native_run)
            try:
                original_through_tunnel(args, ssh)
            except Exception as error:
                (args.out / "original-error.txt").write_text(str(error) + "\n")
            run(transfer + ["--include=refs/", "--include=refs/**",
                            "--include=original-error.txt", "--exclude=*", str(args.out) + "/",
                            f"{args.mac}:{scratch}/output/"])
            run(ssh + ["touch " + shlex.quote(f"{scratch}/output/original-ready")])
            result = native.result()
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
