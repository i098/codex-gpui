import contextlib
import importlib.util
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location("qa_sync", Path(__file__).with_name("qa-sync.py"))
sync = importlib.util.module_from_spec(spec)
spec.loader.exec_module(sync)


class CandidateTests(unittest.TestCase):
    def setUp(self):
        self.stage = Path(tempfile.mkdtemp(prefix="cgx-qa-sync-test-", dir="/tmp"))
        self.profile = Path(tempfile.mkdtemp(prefix="cgx-qa-smoke-test-", dir="/tmp"))
        self.addCleanup(sync.remove_temporary, self.stage, "cgx-qa-sync-")
        self.addCleanup(sync.remove_temporary, self.profile, "cgx-qa-smoke-")
        self.bundle = self.stage / "Codex GPUI QA.app"
        self.bundle.mkdir()
        self.work = self.profile / "work.txt"
        self.work.write_text("preserve this draft")
        self.candidate = dict(bundle=str(self.bundle), stage=str(self.stage),
                              smoke_profile=str(self.profile), revision="a" * 40)

    @contextlib.contextmanager
    def owner(self, ready=True, active=False, revision=None):
        state = dict(ready=ready, active=active, revision=revision or self.candidate["revision"],
                     initialized=True, signed_out=True, profile=str(self.profile.resolve()),
                     executable=str((self.bundle / "Contents/MacOS/codex-gpui").resolve()))
        child = '''import fcntl,json,os,select,sys,time
from pathlib import Path
profile=Path(sys.argv[1]); state=json.loads(sys.argv[2]); state["pid"]=os.getpid()
with (profile/"session.lock").open("a+") as lock:
 fcntl.flock(lock,fcntl.LOCK_EX)
 state["time"]=time.time(); (profile/"runtime.json").write_text(json.dumps(state))
 print("ready",flush=True)
 while True:
  if select.select([sys.stdin],[],[],0.01)[0]: break
  request=profile/"restart.json"
  if request.exists():
   value=json.loads(request.read_text())
   if state["ready"] and not state["active"] and all(value[k]==state[k] for k in ("pid","revision","executable")): break
'''
        process = subprocess.Popen([sys.executable, "-c", child, str(self.profile), json.dumps(state)],
                                   stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True)
        try:
            self.assertEqual(process.stdout.readline().strip(), "ready")
            native = lambda: dict(frontmost=7, apps=[] if process.poll() is not None else [
                dict(pid=process.pid, bundle=str(self.bundle), executable=state["executable"], active=active)])
            with patch.object(sync, "native_state", native):
                yield process
        finally:
            if process.poll() is None:
                process.stdin.write("stop\n")
                process.stdin.flush()
            process.wait(timeout=5)
            process.stdin.close()
            process.stdout.close()

    def test_superseded_idle_owner_closes_before_removal(self):
        with self.owner() as process:
            self.assertTrue(sync.discard_candidate(self.candidate))
            self.assertEqual(process.wait(timeout=5), 0)
            self.assertFalse(self.stage.exists())
            self.assertFalse(self.profile.exists())

    def test_busy_and_foreground_owners_keep_their_work(self):
        for state in (dict(ready=False), dict(active=True)):
            with self.subTest(**state), self.owner(**state) as process:
                self.assertFalse(sync.discard_candidate(self.candidate))
                self.assertIsNone(process.poll())
                self.assertEqual(self.work.read_text(), "preserve this draft")
                self.assertTrue(self.bundle.exists())
                self.assertFalse((self.profile / "restart.json").exists())

    def test_native_foreground_overrides_an_inactive_heartbeat(self):
        with self.owner() as process:
            native = sync.native_state()
            native["frontmost"] = process.pid
            native["apps"][0]["active"] = True
            with patch.object(sync, "native_state", return_value=native):
                self.assertFalse(sync.discard_candidate(self.candidate))
            self.assertIsNone(process.poll())
            self.assertEqual(self.work.read_text(), "preserve this draft")
            self.assertFalse((self.profile / "restart.json").exists())

    def test_wrong_runtime_revision_does_not_close_owner(self):
        with self.owner(revision="b" * 40) as process:
            with self.assertRaisesRegex(RuntimeError, "identity is not verified"):
                sync.discard_candidate(self.candidate)
            self.assertIsNone(process.poll())
            self.assertEqual(self.work.read_text(), "preserve this draft")

    def test_missing_native_record_does_not_bypass_profile_lock(self):
        with self.owner(), patch.object(sync, "native_state", return_value={"apps": []}):
            self.assertFalse(sync.discard_candidate(self.candidate))
            self.assertEqual(self.work.read_text(), "preserve this draft")

    def test_dead_runtime_metadata_does_not_own_profile(self):
        (self.profile / "runtime.json").write_text('{"pid": 99999999}')
        with patch.object(sync, "native_state", return_value={"apps": []}):
            self.assertTrue(sync.discard_candidate(self.candidate))
        self.assertFalse(self.profile.exists())

    def test_completed_install_keeps_a_protected_stage_record(self):
        updater = self.stage / "updater"
        updater.mkdir()
        pending = updater / "pending.json"
        self.candidate["digest"] = "verified-by-bundle-boundary"
        sync.atomic_json(pending, self.candidate)
        with self.owner(ready=False) as process, \
                patch.object(sync, "UPDATER", updater), \
                patch.object(sync, "verify_bundle", return_value=self.candidate), \
                patch.object(sync, "git", return_value="updater script"):
            self.assertTrue(sync.finish_activation(self.candidate))
            self.assertIsNone(process.poll())
            self.assertEqual(json.loads(pending.read_text()), self.candidate)
            self.assertEqual(self.work.read_text(), "preserve this draft")

    def test_current_installation_restarts_once_without_rebuilding(self):
        updater = self.stage / "updater"
        updater.mkdir()
        owners = []
        launch = sync.launch
        with contextlib.ExitStack() as stack:
            stack.enter_context(patch.multiple(sync, SOURCE=self.stage, UPDATER=updater,
                                              INSTALLED=self.bundle, PROFILE=self.profile))
            stack.enter_context(patch.object(sync, "latest_main", return_value=self.candidate["revision"]))
            stack.enter_context(patch.object(sync, "current_installation", return_value=self.candidate))
            stack.enter_context(patch.object(sync, "native_state", return_value={"frontmost": 7, "apps": []}))
            stack.enter_context(patch.object(sync, "build", side_effect=AssertionError("current main must not rebuild")))
            stack.enter_context(patch.object(sync, "launch",
                side_effect=lambda bundle, revision: launch(bundle, revision, profile=self.profile)))
            def open_fixture(*args):
                self.assertEqual(args[0], "/usr/bin/open")
                owners.append(stack.enter_context(self.owner()))
            stack.enter_context(patch.object(sync, "run", side_effect=open_fixture))
            sync.update()
            self.assertEqual(len(owners), 1)
            process = owners[0]
            self.assertIsNone(process.poll())
            sync.update()
            self.assertEqual(len(owners), 1)
            self.assertIsNone(process.poll())
            self.assertEqual(self.work.read_text(), "preserve this draft")
            self.assertTrue(sync.close_ready(self.profile, self.bundle, self.candidate["revision"]))
            self.assertEqual(process.wait(timeout=5), 0)


if __name__ == "__main__":
    unittest.main()
