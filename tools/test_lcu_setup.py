import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest


SCRIPT = Path(__file__).with_name("lcu-setup.sh")


class LcuSetupSafety(unittest.TestCase):
    def test_rejects_invalid_home_and_unverified_download(self):
        with tempfile.TemporaryDirectory(prefix="lcu-setup-test-") as directory:
            root = Path(directory)
            binary = root / "bin"
            binary.mkdir()
            uname = binary / "uname"
            uname.write_text('#!/bin/sh\ncase "$1" in -s) echo Darwin;; -m) echo arm64;; esac\n')
            uname.chmod(0o755)
            curl = binary / "curl"
            curl.write_text('#!/bin/sh\nwhile [ "$1" != -o ]; do shift; done\nprintf corrupt > "$2"\n')
            curl.chmod(0o755)
            env = {**os.environ, "HOME": str(root), "PATH": f"{binary}:{os.environ['PATH']}"}
            for home in ("", "relative"):
                result = subprocess.run(["bash", str(SCRIPT)], env={**env, "CODEX_HOME": home}, capture_output=True, text=True)
                self.assertNotEqual(result.returncode, 0)
                self.assertIn("CODEX_HOME must be", result.stderr)
            home = root / "codex"
            home.mkdir()
            config = home / "config.toml"
            config.write_text('model = "keep-me"\n')
            result = subprocess.run(["bash", str(SCRIPT)], env={**env, "CODEX_HOME": str(home)}, capture_output=True, text=True)
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("FAILED", result.stdout)
            self.assertEqual(config.read_text(), 'model = "keep-me"\n')
            self.assertFalse((home / "computer-use").exists())
            release = home / "computer-use/lcu/current"
            (release / "bin").mkdir(parents=True)
            launcher = release / "bin/lcu"
            launcher.write_text('#!/bin/sh\necho "lcu 0.11.5 (older installation)"\n')
            launcher.chmod(0o755)
            result = subprocess.run(["bash", str(SCRIPT)], env={**env, "CODEX_HOME": str(home)}, capture_output=True, text=True)
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("FAILED", result.stdout)
            self.assertEqual(config.read_text(), 'model = "keep-me"\n')

    def test_rejects_a_mismatched_release_manifest(self):
        with tempfile.TemporaryDirectory(prefix="lcu-version-test-") as directory:
            root = Path(directory)
            binary = root / "bin"
            binary.mkdir()
            uname = binary / "uname"
            uname.write_text('#!/bin/sh\ncase "$1" in -s) echo Darwin;; -m) echo arm64;; esac\n')
            uname.chmod(0o755)
            release = root / "computer-use/lcu/current"
            (release / "bin").mkdir(parents=True)
            launcher = release / "bin/lcu"
            launcher.write_text('#!/bin/sh\necho "lcu 0.11.6 (invalid manifest)"\n')
            launcher.chmod(0o755)
            (release / "node-path").write_text(shutil.which("node") + "\n")
            (release / "bundle.json").write_text('{"version":"0.11.5"}')
            env = {**os.environ, "HOME": str(root), "CODEX_HOME": str(root), "PATH": f"{binary}:{os.environ['PATH']}"}
            result = subprocess.run(["bash", str(SCRIPT)], env=env, capture_output=True, text=True)
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("Expected LCU 0.11.6; found 0.11.5.", result.stderr)
            self.assertFalse((root / "config.toml").exists())


if __name__ == "__main__":
    unittest.main()
