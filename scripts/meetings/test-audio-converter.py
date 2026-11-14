#!/usr/bin/env python3
"""Exercise the production Swift converter with synthetic PCM and device drift."""
from pathlib import Path
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[2]

with tempfile.TemporaryDirectory(prefix="handy-meetings-converter-") as directory:
    directory = Path(directory)
    source = directory / "tests.swift"
    source.write_text((ROOT / "src-tauri/swift/meeting_capture.swift").read_text()
                      + "\n" + (ROOT / "scripts/meetings/audio-converter-tests.swift").read_text())
    executable = directory / "tests"
    subprocess.run(["xcrun", "swiftc", "-parse-as-library", "-O",
                    "-import-objc-header", str(ROOT / "src-tauri/swift/meeting_capture_bridge.h"),
                    str(source), "-o", str(executable)], check=True)
    subprocess.run([str(executable)], check=True)
