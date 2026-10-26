import fcntl
import os
from pathlib import Path
import subprocess
import sys
import time

if len(sys.argv) < 2:
    print("Usage: tools/route1/lease.sh <capture command> [args...]", file=sys.stderr)
    raise SystemExit(64)

prefix = Path(os.environ.get("R1_CAPTURE_LOCK", "/sdc-disk/tmp/cgx/cgx-capture-service/capture.lock"))
endpoints = os.environ.get(
    "R1_CAPTURE_CDP_URLS", "http://127.0.0.1:29484,http://127.0.0.1:29485,http://127.0.0.1:29486",
).split(",")
wait_seconds = float(os.environ.get("R1_CAPTURE_WAIT", "1800"))
prefix.parent.mkdir(parents=True, exist_ok=True)
with open(f"{prefix}.scheduler", "a+") as scheduler:
    fcntl.flock(scheduler, fcntl.LOCK_EX)
    state = Path(f"{prefix}.next")
    value = state.read_text().strip() if state.exists() else "0"
    start = int(value or "0") % len(endpoints)
    state.write_text(str((start + 1) % len(endpoints)))

deadline = time.monotonic() + wait_seconds
while True:
    for offset in range(len(endpoints)):
        slot = (start + offset) % len(endpoints)
        fd = os.open(f"{prefix}.{slot}", os.O_CREAT | os.O_RDWR, 0o600)
        try:
            fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError:
            os.close(fd)
            continue
        env = {**os.environ, "R1_CDP": endpoints[slot].strip()}
        raise SystemExit(subprocess.run(
            ["timeout", "--kill-after=5", os.environ.get("R1_CAPTURE_TIMEOUT", "180"), *sys.argv[1:]],
            env=env, pass_fds=(fd,),
        ).returncode)
    remaining = deadline - time.monotonic()
    if remaining <= 0:
        raise SystemExit(75)
    time.sleep(min(0.1, remaining))
