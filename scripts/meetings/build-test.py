#!/usr/bin/env python3
"""Build the isolated app using the existing signing identity without printing it."""
import json
import os
from pathlib import Path
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[2]
SIGNING_CONFIG = Path("/Users/cyrillegrossholtz/Documents/dev/AI/HandyFork/src-tauri/tauri.screenshot-test.conf.json")

def main():
    existing = json.loads(SIGNING_CONFIG.read_text())
    identity = existing["bundle"]["macOS"]["signingIdentity"]
    if not identity or identity == "-":
        raise SystemExit("The existing test configuration needs a stable signing identity.")
    config = json.loads((ROOT / "src-tauri/tauri.meetings-dev.conf.json").read_text())
    # Reuse the screenshot shell explicitly requested by Cyrille. The runtime
    # meeting profile still isolates data and IPC before Tauri initializes.
    config["identifier"] = existing["identifier"]
    config["bundle"].setdefault("macOS", {})["signingIdentity"] = identity
    with tempfile.NamedTemporaryFile(mode="w", suffix=".json", prefix="handy-meetings-build-", delete=False) as handle:
        os.chmod(handle.name, 0o600)
        json.dump(config, handle)
        path = handle.name
    try:
        log_path = Path("/private/tmp/handy-meetings-build.log")
        print(f"Build log: {log_path}", flush=True)
        with log_path.open("w") as log:
            build = subprocess.run(["bun", "run", "tauri", "build", "--debug", "--config", path, "--bundles", "app"], cwd=ROOT, stdout=log, stderr=subprocess.STDOUT)
        if build.returncode:
            raise SystemExit(f"Build failed with exit code {build.returncode}; inspect the build log without exposing signing identity.")
    finally:
        os.unlink(path)
    bundle = ROOT / "src-tauri/target/debug/bundle/macos/Handy Meetings Dev.app"
    subprocess.run(["codesign", "--verify", "--deep", "--strict", str(bundle)], check=True)
    print(bundle)

if __name__ == "__main__":
    main()
