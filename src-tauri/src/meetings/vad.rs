//! Skip inference only when an entire archived window contains no detected voice.
//! The archive is never trimmed, and a retry can bypass this optimization.
use crate::audio_toolkit::vad::VoiceActivityDetector;

pub(super) fn contains_speech(
    detector: &mut impl VoiceActivityDetector,
    audio: &[f32],
) -> anyhow::Result<bool> {
    detector.reset();
    let size = detector.frame_samples();
    let mut tail = vec![0.0; size];
    for chunk in audio.chunks(size) {
        let frame = if chunk.len() == size {
            chunk
        } else {
            tail.fill(0.0);
            tail[..chunk.len()].copy_from_slice(chunk);
            &tail
        };
        // No onset-duration requirement: a single voiced frame retains short
        // replies such as "Oui", "Non" and "D'accord" for recognition.
        if detector.is_voice(frame)? {
            return Ok(true);
        }
    }
    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio_toolkit::vad::SileroVad;
    use std::path::Path;

    #[test]
    fn conservative_vad_keeps_short_french_replies() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let mut vad =
            SileroVad::new(root.join("resources/models/silero-vad-v6.2-F32.gguf"), 0.1).unwrap();
        assert!(!contains_speech(&mut vad, &vec![0.0; 80_000]).unwrap());
        for file in ["oui.wav", "non.wav", "accord.wav"] {
            let samples = hound::WavReader::open(root.join("src/meetings/fixtures").join(file))
                .unwrap()
                .into_samples::<i16>()
                .map(|sample| f32::from(sample.unwrap()) / 32768.0)
                .collect::<Vec<_>>();
            assert!(
                contains_speech(&mut vad, &samples).unwrap(),
                "VAD removed {file}"
            );
        }
    }
}
