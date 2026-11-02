#!/usr/bin/env python3
"""Exercise native stream recovery using only the dedicated synthetic player."""
import array
import argparse
import json
from pathlib import Path
import time
import wave

from control import request


def call(operation, **arguments):
    response = request({"operation": operation, **arguments})
    if response["status"] != "ok":
        raise RuntimeError(response.get("error", operation))
    return response["data"]


def has_system_audio(detail, since_sample=0):
    for segment in detail["segments"]:
        if segment["track"] != 1 or segment["start_sample"] < since_sample:
            continue
        with wave.open(segment["path"], "rb") as audio:
            samples = array.array("h", audio.readframes(audio.getnframes()))
        if max((abs(sample) for sample in samples), default=0) > 100:
            return True
    return False


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--microphone", default="BY-V")
    args = parser.parse_args()
    assert call("snapshot")["active_id"] is None, "Finish the preceding test first"
    sources = call("sources")
    players = [app for app in sources["applications"] if app["name"] == "Handy Fixture Player"]
    assert len(players) == 1, "Launch only the dedicated fixture player"
    microphones = [mic for mic in sources["microphones"] if mic["name"] == args.microphone]
    assert len(microphones) == 1, f"Connect the selected microphone: {args.microphone}"
    selection = {"microphone_id": microphones[0]["id"], "application_pid": players[0]["pid"]}
    session = call("start", selection=selection, ai_enabled=False)
    session_id = session["id"]
    try:
        time.sleep(20)
        before = call("detail", id=session_id)
        assert has_system_audio(before), "No known source signal before refresh"
        snapshot = call("snapshot")
        boundary = snapshot["elapsed_ms"] * 16
        for _ in range(3):
            call("refresh_capture")
        time.sleep(25)
        snapshot = call("snapshot")
        assert snapshot["active_id"] == session_id, snapshot["capture_error"]
        assert snapshot["preferences"]["selection"] == selection
        after = call("detail", id=session_id)
        assert has_system_audio(after, boundary), "Source signal did not resume after refresh"
        assert all(stats["dropped_samples"] == 0 for stats in after["track_stats"])
        call("stop")
        completed = call("detail", id=session_id)
        assert completed["session"]["status"] == "completed", completed["session"]
        race_session = call("start", selection=selection, ai_enabled=False)
        call("refresh_capture")
        call("stop")
        assert call("snapshot")["active_id"] is None
        result = {"session_id": session_id, "selection": selection,
                  "source_audio_before_and_after": True,
                  "recovery_notice": snapshot["capture_error"],
                  "track_stats": completed["track_stats"],
                  "stop_during_refresh_session_id": race_session["id"],
                  "scope": "Forced refresh and concurrent stop; physical unplug and Bluetooth route events require separate verification."}
        Path("/private/tmp/handy-meetings-recovery-result.json").write_text(json.dumps(result, indent=2))
        print(json.dumps(result, indent=2))
    finally:
        call("stop")


if __name__ == "__main__":
    main()
