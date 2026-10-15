#!/usr/bin/env python3
"""Run only with the fixture editor focused and clipboard-guard as the parent."""
import json
from pathlib import Path
import subprocess
import time

from control import request

session = request({"operation": "snapshot"})["data"]
session_id = session["active_id"]
assert session_id, "Start a controlled native meeting first"
checks = []
for cancel in (True, False, True):
    assert request({"operation": "dictation"})["status"] == "ok"
    time.sleep(2)
    operation = "cancel_dictation" if cancel else "dictation"
    assert request({"operation": operation})["status"] == "ok"
    time.sleep(3 if cancel else 8)
    state = request({"operation": "snapshot"})["data"]
    assert state["active_id"] == session_id and not state["capture_error"], state
    checks.append({"cancelled": cancel, "elapsed_ms": state["elapsed_ms"],
                   "tracks": state["tracks"], "backlog_ms": state["backlog_ms"]})
assert all(final["received_samples"] > initial["received_samples"]
           for final, initial in zip(state["tracks"], session["tracks"]))
clipboard = subprocess.check_output(["pbpaste"])
result = {"session_id": session_id, "checks": checks, "capture_continued": True,
          "clipboard_sentinel_unchanged": clipboard == b"Handy meetings clipboard test",
          "scope": "Native microphone recording, one finalization and two cancellations. No controlled spoken dictation supplied."}
Path("/private/tmp/handy-meetings-coexistence-result.json").write_text(json.dumps(result, indent=2))
print(json.dumps(result, indent=2))
