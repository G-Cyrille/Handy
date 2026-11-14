#!/usr/bin/env python3
"""Verify the independent native assistant using synthetic data and optional Codex turns."""
import argparse
import json
from pathlib import Path
import subprocess
import time

import control


def execute(operation):
    result = control.request(operation)
    if result["status"] != "ok":
        raise RuntimeError(result["error"])
    return result["data"]


def wait_for_reply(session_id, chat_id):
    deadline = time.monotonic() + 150
    while time.monotonic() < deadline:
        workspace = execute({"operation": "workspace", "id": session_id})
        chat = next(c for c in workspace["discussions"] if c["id"] == chat_id)
        if chat["status"] != "running":
            if chat["error"]:
                raise RuntimeError(chat["error"])
            return chat
        time.sleep(0.3)
    execute({"operation": "cancel_agent", "id": session_id, "chat_id": chat_id})
    raise TimeoutError("Native Codex verification timed out")


def message(session_id, chat_id, text, title="Principal"):
    return execute({"operation": "message", "id": session_id, "request": {
        "chat_id": chat_id, "title": title, "text": text, "provider": "codex",
        "model": "", "effort": "low", "fast": True,
        "segment_ids": [], "capture_ids": [], "purpose": "message",
    }})


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--with-codex", action="store_true", help="Make real requests using the existing Codex configuration")
    args = parser.parse_args()
    control.BUNDLE = control.ROOT / "src-tauri/target/debug/bundle/macos/Handy Meeting Assistant Dev.app"
    control.DATA = Path.home() / "Library/Application Support/com.pais.handy.meeting-assistant-dev"
    snapshot = execute({"operation": "snapshot"})
    if snapshot["active_id"] is not None:
        raise RuntimeError("The independent test profile is recording; leave it intact and rerun later")
    session = execute({"operation": "prepare", "title": "Vérification assistant", "folder": "Tests/Assistant"})
    session_id = session["id"]
    execute({"operation": "edit_workspace", "id": session_id, "edit": {
        "kind": "note", "id": None, "at_ms": 0, "text": "Note du test natif, sans réunion audio."
    }})
    root = Path(snapshot["storage_path"])
    shown = json.loads(subprocess.check_output(["python3", str(control.ROOT / "scripts/meetings/query.py"), "--root", str(root), "show", session_id], text=True))
    assert shown["status"] == "prepared"
    assert len(shown["workspace"]["entries"]) == 1
    report = {"session_id": session_id, "native_note_persisted": True, "read_only_cli": True}
    if args.with_codex:
        message(session_id, "main", "Réponds uniquement HANDY_NATIVE_OK. N’utilise aucun outil.")
        first = wait_for_reply(session_id, "main")
        assert first["messages"][-1]["text"].strip() == "HANDY_NATIVE_OK"
        message(session_id, "main", "Réponds uniquement HANDY_RESUME_OK. N’utilise aucun outil.")
        second = wait_for_reply(session_id, "main")
        assert second["messages"][-1]["text"].strip() == "HANDY_RESUME_OK"
        assert first["native_id"] == second["native_id"]
        workspace = message(session_id, None, "Crée verification-handy.txt dans ton dossier de travail, avec une seule ligne HANDY_AGENT_OK. Utilise un outil local pour écrire et vérifier ce fichier. N’utilise aucun service externe.", "Vérification fichier")
        chat_id = workspace["discussions"][-1]["id"]
        message(session_id, chat_id, "Ajoute exactement une ligne HANDY_STEERED_OK à la fin de verification-handy.txt, puis vérifie les deux lignes. N’utilise aucun service externe.", "Vérification fichier")
        finished = wait_for_reply(session_id, chat_id)
        text = (root / "sessions" / session_id / "verification-handy.txt").read_text()
        assert text.splitlines() == ["HANDY_AGENT_OK", "HANDY_STEERED_OK"], text
        assert not finished.get("turns")
        stopped = execute({"operation": "cancel_agent", "id": session_id, "chat_id": chat_id})
        assert next(c for c in stopped["discussions"] if c["id"] == chat_id)["messages"]
        assert next(c for c in stopped["discussions"] if c["id"] == "main")["status"] == "idle"
        report.update(codex_reply=True, same_native_session=True, agent_wrote_file=True, queued_followup_once=True, independent_stop=True)
    execute({"operation": "open_meeting", "id": session_id})
    assert execute({"operation": "snapshot"})["active_id"] is None
    processes = control.app_processes()
    assert len([p for p in processes if not p["worker"]]) == 1, processes
    report["single_primary_process"] = True
    target = Path("/private/tmp/handy-meeting-assistant-native.json")
    target.write_text(json.dumps(report, ensure_ascii=False, indent=2))
    print(json.dumps(report, ensure_ascii=False, indent=2))


if __name__ == "__main__":
    main()
