use super::*;

#[test]
fn live_session_has_priority_and_vad_results_can_be_retried_without_rewriting_text() {
    let dir = tempfile::tempdir().unwrap();
    let store = ArchiveStore::open(dir.path().to_owned()).unwrap();
    let first = store
        .create_session("mic".into(), "system".into(), false, 0)
        .unwrap();
    let mut writer = store.writer(&first, 0).unwrap();
    writer.push(0, 0, &vec![0.2; WINDOW]).unwrap();
    writer.finish(5_000, STATUS_INTERRUPTED).unwrap();
    let second = store
        .create_session("mic".into(), "system".into(), false, 0)
        .unwrap();
    let mut writer = store.writer(&second, 0).unwrap();
    writer.push(0, 0, &vec![0.3; WINDOW]).unwrap();
    writer.finish(5_000, STATUS_COMPLETED).unwrap();
    let preferred = store.next_pending_for(Some(&second.id)).unwrap().unwrap();
    assert_eq!(preferred.session_id, second.id);
    store.set_no_speech(preferred.id).unwrap();
    store.retry_asr(&second.id).unwrap();
    let forced = store.next_pending_for(Some(&second.id)).unwrap().unwrap();
    assert_eq!(forced.asr_status, "force");
    assert_eq!(forced.text, None);
    store
        .set_transcript(forced.id, "Je ne vais pas envoyer le contrat demain")
        .unwrap();
    store.retry_asr(&second.id).unwrap();
    assert_eq!(
        store.segments(&second.id, 0, 10).unwrap()[0]
            .text
            .as_deref(),
        Some("Je ne vais pas envoyer le contrat demain")
    );
    assert_eq!(store.next_pending().unwrap().unwrap().session_id, first.id);
}
use tempfile::TempDir;

const S: u64 = SAMPLE_RATE as u64;
const ORIGIN: u64 = 1_000_000_000_000;
/// Samples per simulated capture callback (10 ms).
const CHUNK: u64 = 160;

fn archive() -> (TempDir, ArchiveStore) {
    let dir = tempfile::Builder::new()
        .prefix("meetings-archive-")
        .tempdir_in("/tmp")
        .unwrap();
    let store = ArchiveStore::open(dir.path().to_path_buf()).unwrap();
    (dir, store)
}

fn start(store: &ArchiveStore) -> (Session, ArchiveWriter) {
    let session = store
        .create_session("Mic".into(), "System".into(), true, ORIGIN)
        .unwrap();
    let writer = store.writer(&session, ORIGIN).unwrap();
    (session, writer)
}

/// Uptime timestamp of a timeline sample (16 kHz = 62.5 µs per sample).
fn ts(sample: u64) -> u64 {
    ORIGIN + sample * 62_500
}

/// Pushes samples `[from, to)` of a track as 10 ms callbacks at a constant level.
fn feed(w: &mut ArchiveWriter, track: u8, from: u64, to: u64, level: f32) {
    let mut pos = from;
    while pos < to {
        let n = CHUNK.min(to - pos);
        w.push(track, ts(pos), &vec![level; n as usize]).unwrap();
        pos += n;
    }
}

fn find(segments: &[Segment], track: u8, window: u64) -> &Segment {
    segments
        .iter()
        .find(|s| s.track == track && s.start_sample == window * WINDOW_SAMPLES)
        .unwrap_or_else(|| panic!("missing segment t{track} w{window}"))
}

fn assert_level(samples: &[f32], range: std::ops::Range<u64>, level: f32) {
    for i in range {
        let got = samples[i as usize];
        assert!((got - level).abs() < 1e-3, "sample {i}: {got} != {level}");
    }
}

fn write_wav(path: &Path, len: usize, level: f32) {
    let mut w = hound::WavWriter::create(path, wav_spec()).unwrap();
    for _ in 0..len {
        w.write_sample(to_i16(level)).unwrap();
    }
    w.finalize().unwrap();
}

#[test]
fn two_tracks_share_one_timeline_with_silence_gaps_and_short_words() {
    let (_dir, store) = archive();
    let (session, mut w) = start(&store);

    // Microphone talks for 12 s with a 500 ms callback gap at 6.0 s.
    feed(&mut w, TRACK_MICROPHONE, 0, 6 * S, 0.5);
    feed(&mut w, TRACK_MICROPHONE, 6 * S + S / 2, 12 * S, 0.5);
    // System audio only delivers two short words; one crosses the 5 s boundary.
    feed(&mut w, TRACK_SYSTEM, 2 * S, 2 * S + 3 * S / 10, 0.25);
    feed(&mut w, TRACK_SYSTEM, 49 * S / 10, 52 * S / 10, 0.25);
    w.flush_until(11_500).unwrap();
    assert_eq!(store.segments(&session.id, 0, 100).unwrap().len(), 4);
    w.finish(12_000, STATUS_COMPLETED).unwrap();

    let segments = store.segments(&session.id, 0, 100).unwrap();
    let order: Vec<(u64, u8)> = segments.iter().map(|s| (s.start_sample, s.track)).collect();
    assert_eq!(
        order,
        [
            (0, 0),
            (0, 1),
            (80_000, 0),
            (80_000, 1),
            (160_000, 0),
            (160_000, 1)
        ]
    );
    assert!(segments[..4]
        .iter()
        .all(|s| s.sample_count == WINDOW_SAMPLES));
    assert!(segments[4..].iter().all(|s| s.sample_count == 2 * S));

    let mic = store.read_segment(find(&segments, 0, 1)).unwrap();
    assert_level(&mic, 0..S, 0.5);
    assert_level(&mic, S..S + S / 2, 0.0);
    assert_level(&mic, S + S / 2..WINDOW_SAMPLES, 0.5);

    let sys0 = store.read_segment(find(&segments, 1, 0)).unwrap();
    assert_level(&sys0, 0..2 * S, 0.0);
    assert_level(&sys0, 2 * S..2 * S + 4800, 0.25);
    assert_level(&sys0, 2 * S + 4800..78_400, 0.0);
    assert_level(&sys0, 78_400..80_000, 0.25);
    let sys1 = store.read_segment(find(&segments, 1, 1)).unwrap();
    assert_level(&sys1, 0..3200, 0.25);
    assert_level(&sys1, 3200..WINDOW_SAMPLES, 0.0);

    // A window no callback reached is pure padding: nothing to transcribe.
    assert_eq!(find(&segments, 1, 2).text.as_deref(), Some(""));
    assert_eq!(find(&segments, 0, 2).text, None);
    assert_eq!(store.backlog_ms(&session.id).unwrap(), 22_000);

    let stats = store.track_stats(&session.id).unwrap();
    assert_eq!(stats[0].delivered_samples, 23 * S / 2);
    assert_eq!(stats[0].padded_samples, S / 2);
    assert_eq!(stats[1].delivered_samples, 9600);
    assert_eq!(stats[1].padded_samples, 12 * S - 9600);
    assert!(stats
        .iter()
        .all(|s| s.overlapped_samples == 0 && s.dropped_samples == 0));

    let session = store.session(&session.id).unwrap();
    assert_eq!(
        (session.status.as_str(), session.duration_ms),
        (STATUS_COMPLETED, 12_000)
    );
    assert!(
        w.push(0, ts(12 * S), &[0.1]).is_err(),
        "finished writer accepts audio"
    );

    // Stereo export keeps both tracks aligned on the timeline.
    let export = _dir.path().join("export.wav");
    store.export_wav(&session.id, &export).unwrap();
    let reader = hound::WavReader::open(&export).unwrap();
    assert_eq!(
        (reader.spec().channels, reader.duration()),
        (2, 12 * S as u32)
    );
    let frames: Vec<i16> = reader.into_samples().map(Result::unwrap).collect();
    let frame = |sample: u64| (frames[2 * sample as usize], frames[2 * sample as usize + 1]);
    assert_eq!(frame(21 * S / 10), (to_i16(0.5), to_i16(0.25)));
    assert_eq!(frame(62 * S / 10), (0, 0));
    assert_eq!(frame(5 * S + 100), (to_i16(0.5), to_i16(0.25)));
}

#[test]
fn overlapping_late_and_out_of_range_samples_are_counted_not_written() {
    let (_dir, store) = archive();
    let (session, mut w) = start(&store);

    feed(&mut w, TRACK_MICROPHONE, 0, S, 0.5);
    // Same-track overlap: already written audio wins.
    w.push(0, ts(S / 2), &vec![0.9; (S / 2) as usize]).unwrap();
    // Small timestamp jitter is treated as contiguous audio.
    w.push(0, ts(S - 100), &[0.5; 160]).unwrap();
    // Bogus timestamp far in the future.
    w.push(0, ts(100 * S), &[0.5; 160]).unwrap();
    // Callback starting 80 samples before the origin.
    w.push(1, ORIGIN - 80 * 62_500, &[0.3; 160]).unwrap();
    assert_eq!(w.buffered_samples(), (S + 160 + 80) as usize);

    w.flush_until(5_000).unwrap();
    // Late callback straddling the sealed boundary: only the unsealed part is kept.
    w.push(0, ts(78_400), &[0.7; 3200]).unwrap();
    w.finish(5_200, STATUS_COMPLETED).unwrap();

    let stats = store.track_stats(&session.id).unwrap();
    assert_eq!(stats[0].delivered_samples, S + 160 + 1600);
    assert_eq!(stats[0].overlapped_samples, S / 2);
    assert_eq!(stats[0].dropped_samples, 160 + 1600);
    assert_eq!(
        (stats[1].delivered_samples, stats[1].dropped_samples),
        (80, 80)
    );

    let segments = store.segments(&session.id, 0, 100).unwrap();
    let mic0 = store.read_segment(find(&segments, 0, 0)).unwrap();
    assert_level(&mic0, 0..S + 160, 0.5);
    assert_level(&mic0, S + 160..WINDOW_SAMPLES, 0.0);
    let mic1 = store.read_segment(find(&segments, 0, 1)).unwrap();
    assert_eq!(mic1.len(), 3200);
    assert_level(&mic1, 0..1600, 0.7);
    assert_level(&mic1, 1600..3200, 0.0);
    let sys0 = store.read_segment(find(&segments, 1, 0)).unwrap();
    assert_level(&sys0, 0..80, 0.3);
}

#[test]
fn memory_stays_bounded_without_flushes_and_across_long_gaps() {
    let (_dir, store) = archive();
    let (session, mut w) = start(&store);
    let bound = 2 * WINDOW;

    feed(&mut w, TRACK_MICROPHONE, 0, 30 * S, 0.5);
    assert!(w.buffered_samples() <= bound);
    let mic_windows = |store: &ArchiveStore| {
        let segments = store.segments(&session.id, 0, 500).unwrap();
        segments.iter().filter(|s| s.track == 0).count()
    };
    assert_eq!(mic_windows(&store), 4);

    // A 50 s hole is padded incrementally, never as one allocation.
    w.push(0, ts(80 * S), &[0.5; 160]).unwrap();
    assert!(w.buffered_samples() <= bound);
    assert_eq!(w.stats(0).unwrap().padded_samples, 50 * S);

    w.flush_until(85_000).unwrap();
    let segments = store.segments(&session.id, 0, 500).unwrap();
    let system: Vec<_> = segments.iter().filter(|s| s.track == 1).collect();
    assert_eq!(system.len(), 17);
    assert!(system.iter().all(|s| s.text.as_deref() == Some("")));
    assert_eq!(store.next_pending().unwrap().unwrap().track, 0);
    assert_eq!(store.session(&session.id).unwrap().duration_ms, 85_000);
}

#[test]
fn restart_recovers_committed_and_verified_audio_only() {
    let (dir, store) = archive();
    let (session, mut w) = start(&store);
    feed(&mut w, TRACK_MICROPHONE, 0, 12 * S, 0.5);
    w.flush_until(10_000).unwrap();
    // Process dies: the writer vanishes with 2 s still buffered.
    drop(w);

    let session_dir = store.session_dir(&session.id).unwrap();
    // Crash after finalize but before rename: complete temp file.
    write_wav(&session_dir.join(".t0-000002.wav.tmp"), WINDOW, 0.4);
    // Crash mid-write: header promises more samples than the file holds.
    let torn = session_dir.join(".t1-000002.wav.tmp");
    write_wav(&torn, WINDOW, 0.4);
    File::options()
        .write(true)
        .open(&torn)
        .unwrap()
        .set_len(1000)
        .unwrap();
    // Crash between rename and index commit: unindexed final WAV.
    write_wav(&session_dir.join("t1-000003.wav"), WINDOW, 0.2);
    // Files the archive does not own are left alone.
    fs::write(session_dir.join("notes.txt"), "keep").unwrap();
    fs::write(session_dir.join(".other.tmp"), "keep").unwrap();

    let store = ArchiveStore::open(dir.path().to_path_buf()).unwrap();
    let recovered = store.session(&session.id).unwrap();
    assert_eq!(recovered.status, STATUS_INTERRUPTED);
    assert_eq!(recovered.duration_ms, 20_000);

    let segments = store.segments(&session.id, 0, 100).unwrap();
    let mut keys: Vec<(u8, u64)> = segments
        .iter()
        .map(|s| (s.track, s.start_sample / WINDOW_SAMPLES))
        .collect();
    keys.sort();
    assert_eq!(keys, [(0, 0), (0, 1), (0, 2), (1, 0), (1, 1), (1, 3)]);
    assert_level(
        &store.read_segment(find(&segments, 0, 2)).unwrap(),
        0..WINDOW_SAMPLES,
        0.4,
    );
    assert!(!torn.exists());
    assert!(!session_dir.join(".t0-000002.wav.tmp").exists());
    assert!(session_dir.join("notes.txt").exists());
    assert!(session_dir.join(".other.tmp").exists());

    // Interrupted sessions keep feeding ASR but cannot be written again.
    assert_eq!(
        store.next_pending().unwrap().unwrap().session_id,
        session.id
    );
    assert!(store.writer(&recovered, ORIGIN).is_err());

    // Recovery is idempotent and a new session starts cleanly next to the old one.
    let store = ArchiveStore::open(dir.path().to_path_buf()).unwrap();
    assert_eq!(store.segments(&session.id, 0, 100).unwrap().len(), 6);
    let (next, mut w) = start(&store);
    assert_ne!(next.id, session.id);
    feed(&mut w, TRACK_SYSTEM, 0, S, 0.1);
    w.finish(1_000, STATUS_COMPLETED).unwrap();
    let ids: Vec<String> = store
        .list_sessions()
        .unwrap()
        .into_iter()
        .map(|s| s.id)
        .collect();
    assert_eq!(ids, [next.id.clone(), session.id.clone()]);
    assert_eq!(store.segments(&next.id, 0, 100).unwrap().len(), 2);
}

#[cfg(unix)]
#[test]
fn disk_errors_propagate_and_keep_audio() {
    use std::os::unix::fs::PermissionsExt;

    /// Restores write access so the temp dir can be cleaned up even if an assert fails.
    struct Writable(PathBuf);
    impl Drop for Writable {
        fn drop(&mut self) {
            let _ = fs::set_permissions(&self.0, fs::Permissions::from_mode(0o755));
        }
    }

    let (_dir, store) = archive();
    let (session, mut w) = start(&store);
    let session_dir = store.session_dir(&session.id).unwrap();
    feed(&mut w, TRACK_MICROPHONE, 0, 7 * S, 0.5);
    w.flush_until(5_000).unwrap();

    let guard = Writable(session_dir.clone());
    fs::set_permissions(&session_dir, fs::Permissions::from_mode(0o555)).unwrap();
    if File::create(session_dir.join("probe")).is_ok() {
        eprintln!("skipping: permissions are not enforced (running as root?)");
        return;
    }

    feed(&mut w, TRACK_MICROPHONE, 7 * S, 10 * S, 0.5);
    let err = w.flush_until(10_000).unwrap_err();
    assert!(err.contains("create segment"), "{err}");

    // The writer keeps buffering until two windows are pending, then refuses audio.
    let mut pos = 10 * S;
    let refused = loop {
        if let Err(e) = w.push(0, ts(pos), &[0.5; 160]) {
            break e;
        }
        pos += 160;
        assert!(pos < 20 * S, "push never failed");
    };
    assert!(refused.contains("create segment"), "{refused}");
    assert_eq!(pos, 15 * S);
    assert!(w.buffered_samples() <= 2 * WINDOW);
    assert_eq!(w.stats(0).unwrap().dropped_samples, 160);

    // Audio sealed before the failure is untouched.
    let segments = store.segments(&session.id, 0, 100).unwrap();
    assert_eq!(segments.len(), 2);
    assert_level(
        &store.read_segment(find(&segments, 0, 0)).unwrap(),
        0..WINDOW_SAMPLES,
        0.5,
    );

    // Once the disk recovers, the retained window is sealed with its real audio.
    drop(guard);
    w.flush_until(10_000).unwrap();
    let segments = store.segments(&session.id, 0, 100).unwrap();
    assert_level(
        &store.read_segment(find(&segments, 0, 1)).unwrap(),
        0..WINDOW_SAMPLES,
        0.5,
    );
    w.finish(15_000, STATUS_COMPLETED).unwrap();
    assert!(list_files(&session_dir)
        .unwrap()
        .iter()
        .all(|f| !f.ends_with(".tmp")));
    assert_eq!(
        store.track_stats(&session.id).unwrap()[0].dropped_samples,
        160
    );
}

#[test]
fn asr_queue_follows_timeline_across_sessions() {
    let (_dir, store) = archive();
    let (first, mut w) = start(&store);
    feed(&mut w, TRACK_MICROPHONE, 0, 7 * S, 0.5);
    feed(&mut w, TRACK_SYSTEM, 0, 7 * S, 0.5);
    w.finish(7_000, STATUS_COMPLETED).unwrap();
    let (second, mut w) = start(&store);
    feed(&mut w, TRACK_MICROPHONE, 0, S, 0.5);
    w.finish(1_000, STATUS_COMPLETED).unwrap();

    let mut done = Vec::new();
    while let Some(segment) = store.next_pending().unwrap() {
        done.push((
            segment.session_id.clone(),
            segment.start_sample,
            segment.track,
        ));
        if done.len() == 2 {
            store.set_asr_error(segment.id, "model crashed").unwrap();
        } else {
            store.set_transcript(segment.id, "hello").unwrap();
        }
    }
    let expected = [
        (first.id.clone(), 0, 0),
        (first.id.clone(), 0, 1),
        (first.id.clone(), WINDOW_SAMPLES, 0),
        (first.id.clone(), WINDOW_SAMPLES, 1),
        (second.id.clone(), 0, 0),
    ];
    assert_eq!(done, expected);
    assert_eq!(store.backlog_ms(&first.id).unwrap(), 0);

    store.retry_asr(&first.id).unwrap();
    let retried = store.next_pending().unwrap().unwrap();
    assert_eq!((retried.track, retried.asr_error.as_deref()), (1, None));
    assert_eq!(store.backlog_ms(&first.id).unwrap(), 5_000);

    let page = store.segments(&first.id, 1, 2).unwrap();
    assert_eq!(
        page.iter().map(|s| s.text.as_deref()).collect::<Vec<_>>(),
        [None, Some("hello")]
    );

    // Reads resolve the file from the index, never from the caller's path.
    let forged = Segment {
        path: "/etc/passwd".into(),
        ..retried.clone()
    };
    assert_eq!(store.read_segment(&forged).unwrap().len(), WINDOW);
    assert!(store
        .read_segment(&Segment {
            id: 9999,
            ..retried
        })
        .is_err());

    store.set_ai_enabled(&second.id, false).unwrap();
    store.fail_session(&second.id, "disk full").unwrap();
    let second = store.session(&second.id).unwrap();
    assert!(!second.ai_enabled);
    assert_eq!(
        (second.status.as_str(), second.error.as_deref()),
        (STATUS_FAILED, Some("disk full"))
    );
    assert!(store.session("missing").is_err());
    assert!(store.set_transcript(9999, "x").is_err());
}
