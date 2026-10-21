import os
from pathlib import Path
import subprocess
import sys
import tempfile


def check_lease():
    lease = Path(__file__).with_name("lease.sh")
    with tempfile.TemporaryDirectory(prefix="capture-lease-", dir=os.environ.get("TMPDIR")) as directory:
        env = {**os.environ, "R1_CAPTURE_LOCK": str(Path(directory) / "capture.lock")}
        holder = subprocess.Popen(
            [str(lease), sys.executable, "-c", "print('held', flush=True); input()"],
            env=env, stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True,
        )
        try:
            assert holder.stdout.readline().strip() == "held"
            blocked = subprocess.run(
                [str(lease), "true"], env={**env, "R1_CAPTURE_WAIT": "0.05"}, timeout=5,
            )
            assert blocked.returncode == 75, blocked.returncode
        finally:
            holder.communicate("\n", timeout=5)
        assert holder.returncode == 0, holder.returncode
        cancelled = subprocess.Popen(
            [str(lease), sys.executable, "-c", "print('held', flush=True); input()"],
            env=env, stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True,
        )
        try:
            assert cancelled.stdout.readline().strip() == "held"
            cancelled.terminate()
            cancelled.wait(timeout=5)
            blocked = subprocess.run(
                [str(lease), "true"], env={**env, "R1_CAPTURE_WAIT": "0.05"}, timeout=5,
            )
            assert blocked.returncode == 75, blocked.returncode
        finally:
            cancelled.communicate("\n", timeout=5)
        expired = subprocess.run(
            [str(lease), sys.executable, "-c", "import time; time.sleep(10)"],
            env={**env, "R1_CAPTURE_TIMEOUT": "0.1"}, timeout=10,
        )
        assert expired.returncode == 124, expired.returncode
        released = subprocess.run([str(lease), "true"], env=env, timeout=5)
        assert released.returncode == 0, released.returncode
    print("capture lease: exclusion, cancellation, deadline, and release passed")


if __name__ == "__main__":
    check_lease()
