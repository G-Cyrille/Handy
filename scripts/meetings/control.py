#!/usr/bin/env python3
"""Send one request to the isolated debug app and print its JSON response."""
import argparse
import json
from pathlib import Path
import subprocess
import time
import uuid

ROOT = Path(__file__).resolve().parents[2]
BUNDLE = ROOT / "src-tauri/target/debug/bundle/macos/Handy Meetings Dev.app"
DATA = Path.home() / "Library/Application Support/com.pais.handy.meetings-dev"

def app_processes():
    executable = str(BUNDLE / "Contents/MacOS/handy")
    rows = subprocess.check_output(["ps", "-axo", "pid=,ppid=,rss=,%cpu=,command="], text=True)
    result = []
    for row in rows.splitlines():
        parts = row.split(None, 4)
        if len(parts) == 5 and parts[4].startswith(executable):
            result.append({"pid": int(parts[0]), "ppid": int(parts[1]), "rss_kib": int(parts[2]), "cpu_percent": float(parts[3]), "worker": "--transcribe-worker" in parts[4], "client": "--meeting-control" in parts[4]})
    return result


def request(operation, timeout=30):
    if not any(not item["worker"] and not item["client"] for item in app_processes()):
        raise RuntimeError("Launch Handy Meetings Dev through Finder first; IPC never starts a primary process.")
    request_id = uuid.uuid4().hex
    operation = {**operation, "request_id": request_id}
    response = DATA / "meeting-test-control" / f"{request_id}.json"
    process = subprocess.Popen([str(BUNDLE / "Contents/MacOS/handy"), "--meeting-control", json.dumps(operation)],
                   cwd=ROOT / "src-tauri", stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        if response.exists():
            result = json.loads(response.read_text())
            response.unlink()
            process.wait(timeout=5)
            return result
        time.sleep(0.1)
    raise TimeoutError(f"No response from Handy Meetings Dev for {operation['operation']}")

if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("operation", help='JSON object, e.g. {"operation":"snapshot"}')
    parser.add_argument("--timeout", type=int, default=30)
    args = parser.parse_args()
    print(json.dumps(request(json.loads(args.operation), args.timeout), ensure_ascii=False, indent=2))
