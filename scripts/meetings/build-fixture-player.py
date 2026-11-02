#!/usr/bin/env python3
"""Build a local audio-only source for controlled ScreenCaptureKit tests."""
from pathlib import Path
import plistlib
import subprocess

ROOT = Path(__file__).resolve().parents[2]
BUNDLE = Path("/private/tmp/Handy Fixture Player.app")
executable = BUNDLE / "Contents/MacOS/fixture-player"
executable.parent.mkdir(parents=True, exist_ok=True)
(BUNDLE / "Contents/Info.plist").write_bytes(plistlib.dumps({
    "CFBundleIdentifier": "net.grossholtz.handy.fixture-player",
    "CFBundleName": "Handy Fixture Player",
    "CFBundleExecutable": "fixture-player",
    "CFBundlePackageType": "APPL",
}))
subprocess.run(["xcrun", "swiftc", str(ROOT / "scripts/meetings/fixture-player.swift"),
                "-o", str(executable)], check=True)
subprocess.run(["codesign", "--force", "--sign", "-", str(BUNDLE)], check=True)
print(BUNDLE)
