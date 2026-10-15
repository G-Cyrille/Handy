#!/usr/bin/env python3
"""Measure an already running native test session; never launch or stop Handy."""
import argparse
import array
import json
from pathlib import Path
import shutil
import sqlite3
import time
import wave

from control import app_processes, request


def archive_bytes(root):
    return sum(path.stat().st_size for path in root.rglob("*") if path.is_file())


def snapshot():
    response = request({"operation": "snapshot"})
    if response["status"] != "ok":
        raise RuntimeError(response.get("error", "Snapshot failed"))
    return response["data"]


def asr_progress(root, session_id):
    with sqlite3.connect(f"file:{root / 'archive.sqlite3'}?mode=ro", uri=True) as database:
        counts = dict(database.execute(
            "SELECT asr_status, count(*) FROM segments WHERE session_id = ? GROUP BY asr_status",
            (session_id,),
        ))
        latest = database.execute(
            "SELECT max(start_sample + sample_count) FROM segments "
            "WHERE session_id = ? AND asr_status = 'final'", (session_id,),
        ).fetchone()[0]
    return {"counts": counts, "latest_transcript_end_ms": (latest or 0) / 16}


def fixture_peak(root, session_id):
    with sqlite3.connect(f"file:{root / 'archive.sqlite3'}?mode=ro", uri=True) as database:
        files = database.execute(
            "SELECT file FROM segments WHERE session_id = ? AND track = 1 "
            "ORDER BY start_sample DESC LIMIT 3", (session_id,),
        ).fetchall()
    peak = 0
    for (file,) in files:
        with wave.open(str(root / "sessions" / session_id / file), "rb") as audio:
            samples = array.array("h", audio.readframes(audio.getnframes()))
        peak = max(peak, max((abs(sample) for sample in samples), default=0))
    return peak


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--seconds", type=float, default=4 * 60 * 60)
    parser.add_argument("--interval", type=float, default=30)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--verify-fixture-signal", action="store_true",
                        help="Fail if the known repeating fixture loses its system signal")
    args = parser.parse_args()
    if args.seconds <= 0 or args.interval <= 0:
        parser.error("Durations must be positive")
    initial = snapshot()
    session_id = initial["active_id"]
    if not session_id:
        parser.error("Start a native two-track meeting in Handy Meetings Dev first")
    if args.verify_fixture_signal:
        session = next(item for item in initial["sessions"] if item["id"] == session_id)
        if session["system"] != "Handy Fixture Player":
            parser.error("Signal verification requires the dedicated Handy Fixture Player source")
    args.output.parent.mkdir(parents=True, exist_ok=True)
    started = time.monotonic()
    errors = []
    warnings = set()
    samples = 0
    completed = False
    root = Path(initial["storage_path"])
    try:
        with args.output.open("x") as output:
            while True:
                current = snapshot()
                elapsed = time.monotonic() - started
                detail = request({"operation": "detail", "id": session_id})
                data = detail.get("data", {})
                record = {
                    "wall_time": time.time(),
                    "elapsed_seconds": elapsed,
                    "session_id": session_id,
                    "processes": app_processes(),
                    "archive_bytes": archive_bytes(root),
                    "disk_free_bytes": shutil.disk_usage(root).free,
                    "active_id": current["active_id"],
                    "capture_error": current["capture_error"],
                    "tracks": current["tracks"],
                    "backlog_ms": current["backlog_ms"],
                    "buffered_samples": current["buffered_samples"],
                    "asr_last_duration_ms": current["asr_last_duration_ms"],
                    "asr_last_lag_ms": current["asr_last_lag_ms"],
                    "session": data.get("session"),
                    "track_stats": data.get("track_stats"),
                    "total_segments": data.get("total_segments"),
                    "asr_progress": asr_progress(root, session_id),
                }
                if args.verify_fixture_signal:
                    record["fixture_system_peak_i16"] = fixture_peak(root, session_id)
                output.write(json.dumps(record) + "\n")
                output.flush()
                samples += 1
                print(f"{elapsed:.0f}s backlog={record['backlog_ms']}ms "
                      f"buffer={record['buffered_samples']} archive={record['archive_bytes']}", flush=True)
                if current["active_id"] != session_id:
                    errors.append("Native session stopped or changed")
                    if current["capture_error"]:
                        errors.append(current["capture_error"])
                    break
                if current["capture_error"]:
                    warnings.add(current["capture_error"])
                if detail["status"] != "ok":
                    errors.append(detail.get("error", "Detail failed"))
                    break
                if args.verify_fixture_signal and elapsed > 30 and record["fixture_system_peak_i16"] <= 100:
                    errors.append("Known fixture system audio missing from the last three archive windows")
                    break
                if elapsed >= args.seconds:
                    completed = True
                    break
                time.sleep(min(args.interval, args.seconds - elapsed))
    except KeyboardInterrupt:
        errors.append("Monitoring interrupted; meeting left running")
    except Exception as error:
        errors.append(str(error))
    result = {
        "session_id": session_id,
        "measured_seconds": time.monotonic() - started,
        "requested_seconds": args.seconds,
        "samples": samples,
        "completed": completed,
        "errors": sorted(set(errors)),
        "warnings": sorted(warnings),
        "interpretation": "Telemetry only. Check speech markers, ASR progress and exported audio before accepting endurance.",
    }
    args.output.with_suffix(".summary.json").write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps(result, indent=2))
    return 0 if completed and not errors else 1


if __name__ == "__main__":
    raise SystemExit(main())
