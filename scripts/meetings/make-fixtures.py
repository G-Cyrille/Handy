#!/usr/bin/env python3
"""Create French speech and timeline markers, using only synthetic macOS TTS."""
import json
from pathlib import Path
import subprocess
import wave

OUT = Path("/private/tmp/handy-meetings-fixtures")
PHRASES = [
    (0.5, "Oui"),
    (5.5, "Non"),
    (10.5, "D'accord"),
    (15.5, "Je vais envoyer le contrat demain"),
    (20.5, "Je ne vais pas envoyer le contrat demain"),
    (26.0, "Nous allons vérifier la documentation de cette API avant de décider. Le délai de livraison reste à clarifier. Je propose jeudi, mais nous n'avons pas encore confirmé la date."),
]

def main():
    OUT.mkdir(exist_ok=True)
    audio = bytearray(45 * 16_000 * 2)
    markers = []
    for index, (start, text) in enumerate(PHRASES):
        aiff = OUT / f"phrase-{index}.aiff"
        wav = OUT / f"phrase-{index}.wav"
        subprocess.run(["say", "-v", "Thomas", "-r", "150", "-o", str(aiff), text], check=True)
        subprocess.run(["afconvert", "-f", "WAVE", "-d", "LEI16@16000", "-c", "1", str(aiff), str(wav)], check=True)
        with wave.open(str(wav)) as source:
            frames = source.readframes(source.getnframes())
        if not frames or not any(frames):
            raise RuntimeError(f"TTS returned no audio for phrase {index}")
        offset = int(start * 16_000) * 2
        audio[offset:offset + len(frames)] = frames
        markers.append({"start_ms": int(start * 1000), "duration_ms": len(frames) / 32, "text": text})
    with wave.open(str(OUT / "french.wav"), "wb") as result:
        result.setparams((1, 2, 16_000, 0, "NONE", "not compressed"))
        result.writeframes(audio)
    (OUT / "markers.json").write_text(json.dumps(markers, ensure_ascii=False, indent=2))
    print(OUT / "french.wav")

if __name__ == "__main__":
    main()
