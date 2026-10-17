#!/usr/bin/env python3
"""Run on the Mac. Install only from verified merged main; launchd runs each update in the background."""
import argparse
import fcntl
import hashlib
import json
import os
from pathlib import Path
import plistlib
import shutil
import subprocess
import sys
import tempfile
import time

REPOSITORY = "https://github.com/i098/codex-gpui.git"
IDENTIFIER = "dev.codexgpui.qa"
LABEL = "dev.codexgpui.qa-sync"
HOME = Path.home()
ROOT = HOME / "Library/Application Support/codex-gpui-qa"
UPDATER = ROOT / "updater"
SOURCE = UPDATER / "source"
PROFILE = ROOT / "profile"
INSTALLED = HOME / "Applications/Codex GPUI QA.app"
BACKUP = UPDATER / "last-working.app"


def run(*args, cwd=None, env=None, timeout=120):
    return subprocess.run(args, cwd=cwd, env=env, check=True, text=True,
                          stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=timeout).stdout.strip()


def atomic_json(path, value):
    temporary = path.with_suffix(".tmp")
    temporary.write_text(json.dumps(value, sort_keys=True) + "\n")
    os.replace(temporary, path)


def remove_temporary(path, prefix):
    if path.is_symlink() or path.resolve().parent != Path("/tmp").resolve() or not path.name.startswith(prefix):
        raise RuntimeError("refusing to remove an unexpected temporary path")
    if path.exists():
        shutil.rmtree(path)


def discard_candidate(candidate):
    if process_for(Path(candidate["bundle"])):
        return False
    remove_temporary(Path(candidate["stage"]), "cgx-qa-sync-")
    remove_temporary(Path(candidate["smoke_profile"]), "cgx-qa-smoke-")
    return True


def owned_directory(path):
    if path.is_symlink():
        raise RuntimeError("owned QA directory is a symlink: " + str(path))
    if path.parent.resolve() != path.parent:
        raise RuntimeError("owned QA parent is redirected: " + str(path.parent))
    path.mkdir(mode=0o700, parents=True, exist_ok=True)
    if path.resolve() != path:
        raise RuntimeError("owned QA directory has a redirected parent: " + str(path))
    path.chmod(0o700)


def git(*args, source=SOURCE):
    return run("git", *args, cwd=source)


def clean_source(source):
    if git("remote", "get-url", "origin", source=source) != REPOSITORY:
        raise RuntimeError("unexpected QA source remote")
    if git("status", "--porcelain", "--untracked-files=all", source=source):
        raise RuntimeError("QA source is dirty; refusing to discard it")


def latest_main(source=SOURCE):
    clean_source(source)
    while True:
        git("fetch", "--no-tags", "origin", "refs/heads/main:refs/remotes/origin/main", source=source)
        revision = git("rev-parse", "refs/remotes/origin/main", source=source)
        remote = git("ls-remote", "--exit-code", "origin", "refs/heads/main", source=source).split()[0]
        if revision == remote:
            if git("cat-file", "-t", revision, source=source) != "commit":
                raise RuntimeError("main does not name a commit")
            git("merge-base", "--is-ancestor", "HEAD", revision, source=source)
            return revision


def native_state():
    script = '''ObjC.import("AppKit");
var w=$.NSWorkspace.sharedWorkspace, apps=w.runningApplications, found=[];
for(var i=0;i<apps.count;i++) {
 var a=apps.objectAtIndex(i);
 if(a.bundleIdentifier.js==="dev.codexgpui.qa")
  found.push({pid:Number(a.processIdentifier),bundle:a.bundleURL.path.js,
              executable:a.executableURL.path.js,active:Boolean(a.isActive)});
}
JSON.stringify({frontmost:Number(w.frontmostApplication.processIdentifier),apps:found});'''
    return json.loads(run("/usr/bin/osascript", "-l", "JavaScript", "-e", script, timeout=15))


def process_for(bundle):
    executable = str((bundle / "Contents/MacOS/codex-gpui").resolve())
    matches = [a for a in native_state()["apps"]
               if Path(a["bundle"]).resolve() == bundle.resolve() and a["executable"] == executable]
    if len(matches) > 1:
        raise RuntimeError("multiple processes use the owned QA bundle")
    return matches[0] if matches else None


def runtime(profile, bundle, revision):
    process = process_for(bundle)
    if not process:
        return None
    data = json.loads((profile / "runtime.json").read_text())
    if (data["pid"] != process["pid"] or data["revision"] != revision
            or data["executable"] != process["executable"]
            or Path(data["profile"]).resolve() != profile.resolve()
            or not 0 <= time.time() - data["time"] < 5):
        raise RuntimeError("QA runtime identity is not verified")
    return data


def artifact_digest(bundle):
    digest = hashlib.sha256()
    for path in sorted(bundle.rglob("*")):
        if path.is_symlink():
            raise RuntimeError("unexpected symlink in QA artifact")
        if path.is_file():
            digest.update(str(path.relative_to(bundle)).encode() + b"\0")
            with path.open("rb") as stream:
                for chunk in iter(lambda: stream.read(1024 * 1024), b""):
                    digest.update(chunk)
    return digest.hexdigest()


def verify_bundle(bundle, revision=None, digest=None):
    if bundle.is_symlink():
        raise RuntimeError("QA bundle path is a symlink")
    with (bundle / "Contents/Info.plist").open("rb") as stream:
        info = plistlib.load(stream)
    if info["CFBundleIdentifier"] != IDENTIFIER:
        raise RuntimeError("refusing a bundle that is not QA")
    source = info["CodexGPUIRevision"]
    if len(source) != 40 or any(c not in "0123456789abcdef" for c in source):
        raise RuntimeError("QA artifact has no verified source revision")
    if revision and source != revision:
        raise RuntimeError("QA artifact source does not match main")
    binary = bundle / "Contents/MacOS/codex-gpui"
    run("/usr/bin/codesign", "--verify", "--strict", str(bundle))
    if json.loads(run(str(binary), "--qa-build-info")) != {"qa": True, "revision": source}:
        raise RuntimeError("compiled QA identity mismatch")
    identity = {"revision": source, "digest": artifact_digest(bundle), "bundle": str(bundle)}
    if digest is not None and identity["digest"] != digest:
        raise RuntimeError("QA artifact digest mismatch")
    return identity


def current_installation():
    if not INSTALLED.exists():
        return None
    record = UPDATER / "installed.json"
    if not record.exists():
        raise RuntimeError("refusing an unexpected pre-existing QA app")
    expected = json.loads(record.read_text())
    return verify_bundle(INSTALLED, expected["revision"], expected["digest"])


def close_ready(profile, bundle, revision):
    state = runtime(profile, bundle, revision)
    if state is None:
        return True
    if not state["ready"] or state["active"]:
        return False
    request = profile / "restart.json"
    atomic_json(request, {key: state[key] for key in ("pid", "revision", "executable")})
    try:
        deadline = time.monotonic() + 8
        while time.monotonic() < deadline:
            process = process_for(bundle)
            if process is None:
                return True
            if process["pid"] != state["pid"]:
                raise RuntimeError("QA process changed during the close handshake")
            time.sleep(0.2)
        return False
    finally:
        request.unlink(missing_ok=True)


def launch(bundle, revision, profile=PROFILE, smoke=False):
    before = native_state()["frontmost"]
    existing = process_for(bundle)
    if not existing:
        args = ["/usr/bin/open", "-g", "-n", str(bundle)]
        if smoke:
            args += ["--args", "--qa-smoke-profile", str(profile)]
        run(*args)
    deadline = time.monotonic() + 45
    while time.monotonic() < deadline:
        state = native_state()
        owned = [a for a in state["apps"] if Path(a["bundle"]).resolve() == bundle.resolve()]
        if not existing and any(a["active"] or a["pid"] == state["frontmost"] for a in owned):
            raise RuntimeError("QA took focus during a background launch")
        try:
            current = runtime(profile, bundle, revision)
            if current and current["initialized"] and (existing or not current["active"]):
                print(json.dumps({"event": "running" if existing else "background-start",
                                  "frontmost_before": before, "frontmost_after": state["frontmost"], **current}), flush=True)
                return current
        except (FileNotFoundError, RuntimeError, json.JSONDecodeError):
            pass
        time.sleep(0.3)
    raise RuntimeError("QA did not prove startup before the deadline")


def build(revision):
    git("checkout", "--detach", revision)
    stage = Path(tempfile.mkdtemp(prefix="cgx-qa-sync-", dir="/tmp"))
    environment = os.environ.copy()
    environment.update(CARGO_HOME="/tmp/cgx-mac/cargo", RUSTUP_HOME="/tmp/cgx-mac/rustup",
                       CARGO_TARGET_DIR="/tmp/cgx-mac/target/qa-sync-updater",
                       CARGO_NET_GIT_FETCH_WITH_CLI="true", TMPDIR="/tmp",
                       PATH="/tmp/cgx-mac/cargo/bin:" + str(HOME / ".cargo/bin") + ":/opt/zerobrew/bin:/usr/bin:/bin:/usr/sbin:/sbin")
    log = UPDATER / ("build-" + revision + ".log")
    try:
        with log.open("w") as output:
            subprocess.run(["bash", "tools/bundle.sh", "--assemble", str(stage), "qa", revision],
                           cwd=SOURCE, env=environment, stdout=output, stderr=subprocess.STDOUT,
                           check=True, timeout=1800)
        bundles = list(stage.glob("stage.*/Codex GPUI QA.app"))
        if len(bundles) != 1:
            raise RuntimeError("build did not produce exactly one QA bundle")
        identity = verify_bundle(bundles[0], revision)
    except Exception:
        remove_temporary(stage, "cgx-qa-sync-")
        raise
    identity.update(log=str(log), stage=str(stage), startup_verified=False,
                    smoke_profile=tempfile.mkdtemp(prefix="cgx-qa-smoke-", dir="/tmp"))
    atomic_json(UPDATER / "pending.json", identity)
    return identity


def probe_candidate(candidate):
    bundle = Path(candidate["bundle"])
    profile = Path(candidate["smoke_profile"])
    if process_for(bundle):
        state = runtime(profile, bundle, candidate["revision"])
        if not state or not state["initialized"] or not state["ready"]:
            print("pending: staged QA readiness is unverified", flush=True)
            return False
    else:
        profile.mkdir(mode=0o700, exist_ok=True)
        launch(bundle, candidate["revision"], profile, smoke=True)
    if not close_ready(profile, bundle, candidate["revision"]):
        print("pending: staged QA cannot close safely", flush=True)
        return False
    candidate["startup_verified"] = True
    atomic_json(UPDATER / "pending.json", candidate)
    remove_temporary(profile, "cgx-qa-smoke-")
    return True


def finish_activation(candidate):
    installed = verify_bundle(INSTALLED, candidate["revision"], candidate["digest"])
    atomic_json(UPDATER / "installed.json", installed)
    temporary = UPDATER / "qa-sync.py.tmp"
    temporary.write_text(git("show", candidate["revision"] + ":tools/qa-sync.py") + "\n")
    os.replace(temporary, UPDATER / "qa-sync.py")
    if not discard_candidate(candidate):
        print("retained a running staged QA artifact: " + candidate["stage"], flush=True)
    (UPDATER / "pending.json").unlink(missing_ok=True)
    print(json.dumps({"event": "installed", **installed}), flush=True)
    return True


def recover_unfinished():
    record = UPDATER / "installed.json"
    previous = json.loads(record.read_text()) if record.exists() else None
    if INSTALLED.exists():
        try:
            current_installation()
            return True
        except RuntimeError:
            pending = UPDATER / "pending.json"
            if not pending.exists():
                raise
            candidate = json.loads(pending.read_text())
            verify_bundle(INSTALLED, candidate["revision"], candidate["digest"])
        if process_for(INSTALLED):
            state = runtime(PROFILE, INSTALLED, candidate["revision"])
            if state and state["initialized"]:
                return finish_activation(candidate)
            print("pending: the new QA process has unverified readiness", flush=True)
            return False
    elif previous is None:
        return True
    with (PROFILE / "session.lock").open("a+") as session:
        try:
            fcntl.flock(session, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError:
            return False
        if process_for(INSTALLED):
            return False
        if previous:
            verify_bundle(BACKUP, previous["revision"], previous["digest"])
        if INSTALLED.exists():
            failed = UPDATER / "failed.app"
            if failed.exists():
                verify_bundle(failed)
                shutil.rmtree(failed)
            os.replace(INSTALLED, failed)
        if previous:
            os.replace(BACKUP, INSTALLED)
    if previous:
        launch(INSTALLED, previous["revision"])
    return True


def activate(candidate):
    revision = candidate["revision"]
    bundle = Path(candidate["bundle"])
    verify_bundle(bundle, revision, candidate["digest"])
    previous = current_installation()
    if previous:
        git("merge-base", "--is-ancestor", previous["revision"], revision)
    replacement = UPDATER / "candidate.app"
    if replacement.exists():
        old_candidate = verify_bundle(replacement)
        git("merge-base", "--is-ancestor", old_candidate["revision"], revision)
        shutil.rmtree(replacement)
    run("/usr/bin/ditto", str(bundle), str(replacement))
    verify_bundle(replacement, revision, candidate["digest"])
    if latest_main() != revision:
        return False
    if previous and not close_ready(PROFILE, INSTALLED, previous["revision"]):
        print("pending: QA has active work, a draft, or unverified readiness", flush=True)
        return False
    moved_previous = False
    try:
        with (PROFILE / "session.lock").open("a+") as session:
            try:
                fcntl.flock(session, fcntl.LOCK_EX | fcntl.LOCK_NB)
            except BlockingIOError:
                print("pending: QA reopened before replacement", flush=True)
                return False
            if process_for(INSTALLED):
                return False
            if latest_main() != revision:
                return False
            if BACKUP.exists():
                verify_bundle(BACKUP)
                shutil.rmtree(BACKUP)
            if previous:
                os.replace(INSTALLED, BACKUP)
                moved_previous = True
            INSTALLED.parent.mkdir(exist_ok=True)
            os.replace(replacement, INSTALLED)
            verify_bundle(INSTALLED, revision, candidate["digest"])
        launch(INSTALLED, revision)
    except Exception:
        if process_for(INSTALLED) is None and previous:
            with (PROFILE / "session.lock").open("a+") as session:
                fcntl.flock(session, fcntl.LOCK_EX | fcntl.LOCK_NB)
                if moved_previous:
                    if INSTALLED.exists():
                        os.replace(INSTALLED, replacement)
                    os.replace(BACKUP, INSTALLED)
                atomic_json(UPDATER / "installed.json", previous)
            launch(INSTALLED, previous["revision"])
        raise
    finally:
        if previous and not moved_previous and process_for(INSTALLED) is None:
            launch(INSTALLED, previous["revision"])
    return finish_activation(candidate)


def update():
    if not SOURCE.exists():
        run("git", "clone", "--no-checkout", "--filter=blob:none", REPOSITORY, str(SOURCE))
        git("checkout", "--detach", "origin/main")
    while True:
        revision = latest_main()
        if not recover_unfinished():
            return
        current = current_installation()
        pending = UPDATER / "pending.json"
        candidate = json.loads(pending.read_text()) if pending.exists() else None
        if current and current["revision"] == revision:
            if candidate and candidate["revision"] == revision:
                launch(INSTALLED, revision)
                finish_activation(candidate)
            else:
                print(json.dumps({"event": "synced", **current, "running": runtime(PROFILE, INSTALLED, revision)}), flush=True)
            return
        if candidate and (candidate["revision"] != revision or not Path(candidate["bundle"]).is_dir()):
            if not discard_candidate(candidate):
                print("pending: a staged QA process still owns its profile", flush=True)
                return
            pending.unlink()
            candidate = None
        if candidate is None:
            candidate = build(revision)
        if not candidate["startup_verified"] and not probe_candidate(candidate):
            return
        if latest_main() != revision:
            continue
        if activate(candidate):
            return
        if latest_main() == revision:
            return


def install():
    source = Path(__file__).resolve().parent.parent
    if git("rev-parse", "HEAD", source=source) != latest_main(source):
        raise RuntimeError("install requires a clean checkout of current merged origin/main")
    script = UPDATER / "qa-sync.py"
    shutil.copyfile(__file__, script)
    agent = UPDATER / "agent.plist"
    with agent.open("wb") as stream:
        plistlib.dump({"Label": LABEL, "ProgramArguments": ["/usr/bin/python3", str(script), "once"],
                      "RunAtLoad": True, "StartInterval": 30,
                      "StandardOutPath": str(UPDATER / "sync.log"),
                      "StandardErrorPath": str(UPDATER / "sync.log"),
                      "EnvironmentVariables": {"PATH": "/opt/zerobrew/bin:/usr/bin:/bin:/usr/sbin:/sbin"}}, stream)
    run("/bin/launchctl", "bootstrap", "gui/" + str(os.getuid()), str(agent))
    print("installed QA timer: " + str(agent), flush=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=["install", "once", "status"])
    args = parser.parse_args()
    if sys.platform != "darwin":
        parser.error("QA sync requires macOS")
    for path in (ROOT, UPDATER, PROFILE):
        owned_directory(path)
    with (UPDATER / "lock").open("w") as lock:
        try:
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError:
            print("coalesced: an owned QA update is already running", flush=True)
            return
        if args.command == "install":
            install()
        elif args.command == "status":
            revision = latest_main()
            current = current_installation()
            print(json.dumps({"main": revision, "installed": current,
                              "running": runtime(PROFILE, INSTALLED, current["revision"]) if current else None}), flush=True)
        else:
            update()


if __name__ == "__main__":
    try:
        main()
    except Exception as error:
        print("QA update failed without a forced quit: " + str(error), file=sys.stderr, flush=True)
        sys.exit(1)
