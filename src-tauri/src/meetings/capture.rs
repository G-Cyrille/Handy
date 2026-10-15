//! Audio-only meeting capture. Application selection captures all audio belonging
//! to the selected application PID, including all its windows/tabs.

use serde::{Deserialize, Serialize};
use std::sync::mpsc::SyncSender;

#[derive(Serialize, Deserialize, Clone, Debug, specta::Type)]
pub struct CaptureSources {
    pub microphones: Vec<CaptureDevice>,
    pub applications: Vec<CaptureApplication>,
    pub supported: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug, specta::Type)]
pub struct CaptureDevice {
    pub id: String,
    pub name: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, specta::Type)]
pub struct CaptureApplication {
    pub pid: i32,
    pub name: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, specta::Type)]
pub struct CaptureSelection {
    pub microphone_id: String,
    /// None captures the whole system; Some(pid) captures all that app's tabs.
    pub application_pid: Option<i32>,
}

#[derive(Debug)]
pub struct AudioFrame {
    /// 0 is the explicit microphone, 1 is system/application audio.
    pub track: u8,
    /// Host monotonic uptime in nanoseconds, shared by both tracks.
    pub timestamp_ns: u64,
    /// Mono, 16 kHz PCM, at most 4096 samples per frame.
    pub samples: Vec<f32>,
}

#[derive(Debug)]
pub enum CaptureEvent {
    Audio(AudioFrame),
    Warning(String),
    Error(String),
}

/// List sources, requesting standard Screen & System Audio Recording permission
/// if needed on macOS. Call on a worker thread after an explicit user action.
pub fn sources() -> Result<CaptureSources, String> {
    platform::sources()
}

/// Start one in-process capture. May request standard macOS capture/microphone
/// permissions. Call on a worker thread with a bounded queue sized by the caller.
/// On overflow the recording is failed; later callbacks retry the error event,
/// and stop returns the error even if the consumer never makes room.
pub fn start(
    selection: &CaptureSelection,
    sender: SyncSender<CaptureEvent>,
) -> Result<CaptureHandle, String> {
    platform::start(selection, sender)
}

/// Current host uptime, on the same clock as captured audio PTS on macOS.
pub fn host_time_ns() -> u64 {
    platform::host_time_ns()
}

pub use platform::CaptureHandle;

#[cfg(target_os = "macos")]
mod platform {
    use super::*;
    use std::ffi::{c_char, c_void, CStr, CString};
    use std::ptr::NonNull;
    use std::sync::atomic::{AtomicU8, Ordering};
    use std::sync::mpsc::TrySendError;

    const MAX_FRAME_SAMPLES: usize = 4096;
    const OVERFLOW: u8 = 1;
    const DISCONNECTED: u8 = 2;
    const INVALID_FRAME: u8 = 3;
    const NATIVE_ERROR: u8 = 4;

    type Callback = unsafe extern "C" fn(*mut c_void, u8, u64, *const f32, usize, *const c_char);

    extern "C" {
        fn handy_meeting_sources(result: *mut *mut c_char, error: *mut *mut c_char) -> i32;
        fn handy_meeting_start(
            microphone_id: *const c_char,
            application_pid: i32,
            callback: Callback,
            context: *mut c_void,
            error: *mut *mut c_char,
        ) -> *mut c_void;
        fn handy_meeting_stop(handle: *mut c_void, error: *mut *mut c_char) -> i32;
        #[cfg(debug_assertions)]
        fn handy_meeting_refresh(handle: *mut c_void);
        fn handy_meeting_host_time_ns() -> u64;
        fn handy_meeting_free(string: *mut c_char);
    }

    struct CallbackContext {
        sender: SyncSender<CaptureEvent>,
        failure: AtomicU8,
    }

    fn failure_message(code: u8) -> &'static str {
        match code {
            OVERFLOW => "Meeting audio queue overflowed. The recording is incomplete; stop capture and retry with a faster consumer.",
            DISCONNECTED => "Meeting audio consumer disconnected. The recording is incomplete.",
            INVALID_FRAME => "The native meeting capture bridge returned an invalid or oversized frame.",
            _ => "Native meeting capture failed. Stop capture to retrieve the native error.",
        }
    }

    impl CallbackContext {
        fn fail(&self, code: u8) {
            let _ = self
                .failure
                .compare_exchange(0, code, Ordering::AcqRel, Ordering::Acquire);
        }

        fn retry_error(&self) {
            let code = self.failure.load(Ordering::Acquire);
            if code != 0 {
                let _ = self
                    .sender
                    .try_send(CaptureEvent::Error(failure_message(code).into()));
            }
        }
    }

    // The Swift bridge borrows context for each callback and closes/drains its
    // serial delivery queue before start fails or stop returns. Copies are bounded
    // and try_send never waits for the archive consumer.
    unsafe extern "C" fn receive(
        context: *mut c_void,
        track: u8,
        timestamp_ns: u64,
        samples: *const f32,
        count: usize,
        error: *const c_char,
    ) {
        let Some(context) = context.cast::<CallbackContext>().as_ref() else {
            return;
        };
        if context.failure.load(Ordering::Acquire) != 0 {
            context.retry_error();
            return;
        }
        let event = if !error.is_null() {
            // Native messages are short. Bound the scan/copy even on an ABI bug.
            let length = (0..4096)
                .find(|&index| *error.add(index) == 0)
                .unwrap_or(4096);
            let bytes = std::slice::from_raw_parts(error.cast::<u8>(), length);
            let message = String::from_utf8_lossy(bytes).into_owned();
            if track == u8::MAX {
                // A recovery notice does not invalidate the audio producer.
                let _ = context.sender.try_send(CaptureEvent::Warning(message));
                return;
            }
            context.fail(NATIVE_ERROR);
            CaptureEvent::Error(message)
        } else {
            if samples.is_null() || count == 0 || count > MAX_FRAME_SAMPLES || track > 1 {
                context.fail(INVALID_FRAME);
                context.retry_error();
                return;
            }
            CaptureEvent::Audio(AudioFrame {
                track,
                timestamp_ns,
                samples: std::slice::from_raw_parts(samples, count).to_vec(),
            })
        };
        match context.sender.try_send(event) {
            Ok(()) => {}
            Err(TrySendError::Full(_)) => context.fail(OVERFLOW),
            Err(TrySendError::Disconnected(_)) => context.fail(DISCONNECTED),
        }
    }

    /// Owns the native stream and its callback context. Moving between worker
    /// threads is safe; access and stop still require exclusive ownership.
    pub struct CaptureHandle {
        native: Option<NonNull<c_void>>,
        context: Option<Box<CallbackContext>>,
        stop_error: Option<String>,
    }

    // Swift serializes audio callbacks; native stop waits before context is freed.
    unsafe impl Send for CaptureHandle {}

    impl CaptureHandle {
        #[cfg(debug_assertions)]
        pub fn refresh(&mut self) -> Result<(), String> {
            let native = self.native.ok_or("Meeting capture is stopped")?;
            unsafe { handy_meeting_refresh(native.as_ptr()) };
            Ok(())
        }

        /// Finalize converter tails, close this producer, and wait for callbacks.
        /// Call on a worker thread. Idempotent. A failed/incomplete capture remains an error on later calls.
        pub fn stop(&mut self) -> Result<(), String> {
            if self.native.is_none() {
                return self.stop_error.clone().map_or(Ok(()), Err);
            }
            let result = self.stop_inner();
            self.stop_error = result.as_ref().err().cloned();
            result
        }

        fn stop_inner(&mut self) -> Result<(), String> {
            let native_result = if let Some(native) = self.native.take() {
                let mut error = std::ptr::null_mut();
                let status = unsafe { handy_meeting_stop(native.as_ptr(), &mut error) };
                let message = take_string(error);
                if status != 0 {
                    Err(message.unwrap_or_else(|| "Cannot stop native meeting capture.".into()))
                } else {
                    Ok(())
                }
            } else {
                Ok(())
            };
            // Native stop closed the callback gate, so freeing context also
            // closes our sender even when the caller keeps the stopped handle.
            let context = self.context.take();
            let failure = context
                .as_ref()
                .map_or(0, |context| context.failure.load(Ordering::Acquire));
            if let Some(context) = context.as_ref().filter(|_| failure != 0) {
                context.retry_error();
                // Overflow must remain visible even if native stop also failed.
                if failure != NATIVE_ERROR {
                    return Err(failure_message(failure).into());
                }
            }
            native_result?;
            if failure != 0 {
                return Err(failure_message(failure).into());
            }
            Ok(())
        }
    }

    impl Drop for CaptureHandle {
        fn drop(&mut self) {
            let _ = self.stop();
        }
    }

    fn take_string(pointer: *mut c_char) -> Option<String> {
        if pointer.is_null() {
            return None;
        }
        // Every returned string is an owned, nul-terminated native allocation.
        let value = unsafe { CStr::from_ptr(pointer).to_string_lossy().into_owned() };
        unsafe { handy_meeting_free(pointer) };
        Some(value)
    }

    pub fn sources() -> Result<CaptureSources, String> {
        let mut result = std::ptr::null_mut();
        let mut error = std::ptr::null_mut();
        let status = unsafe { handy_meeting_sources(&mut result, &mut error) };
        let result = take_string(result);
        let error = take_string(error);
        if status != 0 {
            return Err(error.unwrap_or_else(|| "Cannot enumerate meeting capture sources.".into()));
        }
        serde_json::from_str(&result.ok_or("Native capture returned no sources.")?)
            .map_err(|error| format!("Invalid native capture sources: {error}"))
    }

    pub fn start(
        selection: &CaptureSelection,
        sender: SyncSender<CaptureEvent>,
    ) -> Result<CaptureHandle, String> {
        let microphone = CString::new(selection.microphone_id.as_str())
            .map_err(|_| "Microphone ID contains a nul byte.")?;
        if selection.microphone_id.is_empty()
            || selection.application_pid.is_some_and(|pid| pid <= 0)
        {
            return Err("Select a microphone and a valid application PID.".into());
        }
        let mut context = Box::new(CallbackContext {
            sender,
            failure: AtomicU8::new(0),
        });
        let mut error = std::ptr::null_mut();
        let native = unsafe {
            handy_meeting_start(
                microphone.as_ptr(),
                selection.application_pid.unwrap_or(0),
                receive,
                (&mut *context as *mut CallbackContext).cast(),
                &mut error,
            )
        };
        let error = take_string(error);
        let native = NonNull::new(native).ok_or_else(|| {
            error.unwrap_or_else(|| "Cannot start native meeting capture.".into())
        })?;
        Ok(CaptureHandle {
            native: Some(native),
            context: Some(context),
            stop_error: None,
        })
    }

    pub fn host_time_ns() -> u64 {
        unsafe { handy_meeting_host_time_ns() }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn recovery_notice_keeps_the_audio_callback_alive() {
            let (sender, receiver) = std::sync::mpsc::sync_channel(2);
            let mut context = CallbackContext {
                sender,
                failure: AtomicU8::new(0),
            };
            let pointer = (&mut context as *mut CallbackContext).cast();
            let notice = CString::new("Audio capture resumed after a device change").unwrap();
            let samples = [0.25f32; 160];
            unsafe {
                receive(pointer, u8::MAX, 0, std::ptr::null(), 0, notice.as_ptr());
                receive(
                    pointer,
                    1,
                    1_000_000,
                    samples.as_ptr(),
                    samples.len(),
                    std::ptr::null(),
                );
            }
            assert!(matches!(
                receiver.try_recv().unwrap(),
                CaptureEvent::Warning(_)
            ));
            assert!(matches!(
                receiver.try_recv().unwrap(),
                CaptureEvent::Audio(_)
            ));
            assert_eq!(context.failure.load(Ordering::Acquire), 0);
        }
    }
}

#[cfg(not(target_os = "macos"))]
mod platform {
    use super::*;

    pub struct CaptureHandle;

    impl CaptureHandle {
        #[cfg(debug_assertions)]
        pub fn refresh(&mut self) -> Result<(), String> {
            Err("Meeting capture requires macOS 15 or later.".into())
        }

        pub fn stop(&mut self) -> Result<(), String> {
            Ok(())
        }
    }

    impl Drop for CaptureHandle {
        fn drop(&mut self) {
            let _ = self.stop();
        }
    }

    pub fn sources() -> Result<CaptureSources, String> {
        Ok(CaptureSources {
            microphones: Vec::new(),
            applications: Vec::new(),
            supported: false,
        })
    }

    pub fn start(
        _: &CaptureSelection,
        _: SyncSender<CaptureEvent>,
    ) -> Result<CaptureHandle, String> {
        Err("Meeting capture requires macOS 15 or later.".into())
    }

    pub fn host_time_ns() -> u64 {
        static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
        START
            .get_or_init(std::time::Instant::now)
            .elapsed()
            .as_nanos() as u64
    }
}
