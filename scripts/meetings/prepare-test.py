#!/usr/bin/env python3
"""Seed only nonsecret preferences into the separate test bundle's store."""
import hashlib
import json
from pathlib import Path

SOURCE = Path.home() / "Library/Application Support/com.pais.handy.screenshot-test/settings_store.json"
TARGET = Path.home() / "Library/Application Support/com.pais.handy.meetings-dev/settings_store.json"
WHITELIST = ["selected_model", "selected_language", "post_process_provider_id", "post_process_providers", "post_process_models", "ui_language"]

def main():
    if TARGET.exists():
        raise SystemExit("Test settings already exist; keep them intact.")
    original = SOURCE.read_bytes()
    source = json.loads(original)["settings"]
    settings = {key: source[key] for key in WHITELIST if key in source}
    settings.update(autostart_enabled=False, update_checks_enabled=False, always_on_microphone=False,
                    onboarding_completed=True, selected_language="fr", audio_feedback=False,
                    start_hidden=False, mute_while_recording=False)
    TARGET.parent.mkdir(parents=True, exist_ok=True)
    TARGET.write_text(json.dumps({"settings": settings}, ensure_ascii=False, indent=2))
    TARGET.chmod(0o600)
    assert SOURCE.read_bytes() == original, "The source store changed during setup"
    baseline = Path("/private/tmp/handy-meetings-daily-settings-baseline.json")
    baseline.write_text(json.dumps({"source":str(SOURCE),"sha256":hashlib.sha256(original).hexdigest()}))
    print(TARGET.parent)
    print("Nonsecret whitelist only. Credentials and history were not copied.")

if __name__ == "__main__":
    main()
