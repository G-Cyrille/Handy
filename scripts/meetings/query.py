#!/usr/bin/env python3
"""Read-only meeting CLI for people and agents. Never initializes the archive."""
import argparse
import json
import sqlite3
from pathlib import Path
from urllib.parse import quote


def read_json(path, default):
    try:
        return json.loads(path.read_text())
    except FileNotFoundError:
        return default


def query(root, operation, value="", folder="", limit=100):
    root = root.expanduser().resolve()
    connection = sqlite3.connect(f"file:{quote(str(root / 'archive.sqlite3'))}?mode=ro", uri=True)
    connection.row_factory = sqlite3.Row
    with connection:
        rows = connection.execute("SELECT * FROM sessions ORDER BY started_at_ms DESC").fetchall()
        results = []
        for row in rows:
            item = dict(row)
            if not str(item["id"]).isdigit():
                continue
            directory = root / "sessions" / item["id"]
            workspace = read_json(directory / "workspace.json", {})
            insights = read_json(directory / "insights.json", {})
            item.update(title=workspace.get("title", ""), folder="/".join(part.strip() for part in workspace.get("folder", "").split("/") if part.strip()), summary=insights.get("summary", ""))
            if folder and not (item["folder"] == folder or item["folder"].startswith(folder + "/")):
                continue
            if operation == "show" and item["id"] != value:
                continue
            if operation in ("show", "search"):
                segments = [dict(segment) for segment in connection.execute("SELECT id, track, start_sample, sample_count, text, asr_status, asr_error FROM segments WHERE session_id=? ORDER BY start_sample, track", (item["id"],))]
                if operation == "search" and value.casefold() not in json.dumps([item, workspace, segments], ensure_ascii=False).casefold():
                    continue
                if operation == "show":
                    item.update(workspace=workspace, segments=segments, directory=str(directory))
            results.append(item)
            if len(results) >= limit:
                break
        if operation == "show":
            if not results:
                raise ValueError("Unknown meeting")
            return results[0]
        return results


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, required=True, help="Handy meeting archive directory")
    parser.add_argument("--folder", default="")
    parser.add_argument("--limit", type=int, default=100)
    parser.add_argument("operation", choices=["list", "search", "show"])
    parser.add_argument("value", nargs="?", default="")
    args = parser.parse_args()
    if args.operation != "list" and not args.value:
        parser.error("search/show require a query or meeting id")
    try:
        result = query(args.root, args.operation, args.value, args.folder, max(1, min(args.limit, 1000)))
    except (OSError, sqlite3.Error, ValueError) as error:
        parser.exit(1, f"{error}\n")
    print(json.dumps(result, ensure_ascii=False, indent=2))


if __name__ == "__main__":
    main()
