//! One engine, with dictation priority over retryable meeting and file work.
use std::sync::{Condvar, Mutex};

#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum Owner {
    #[default]
    Idle,
    Background,
    Dictation,
}

#[derive(Default)]
struct State {
    owner: Owner,
    dictation_reserved: bool,
}

#[derive(Default)]
pub struct AsrScheduler {
    state: Mutex<State>,
    available: Condvar,
}

pub struct Lease<'a>(&'a AsrScheduler);

impl AsrScheduler {
    /// Reserve before microphone startup. The current bounded meeting segment
    /// finishes first, avoiding a model reload on every dictation.
    pub fn reserve_dictation(&self) {
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .dictation_reserved = true;
    }

    pub fn finish_dictation(&self) {
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .dictation_reserved = false;
        self.available.notify_all();
    }

    pub fn dictation(&self) -> Lease<'_> {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        while state.owner != Owner::Idle {
            state = self
                .available
                .wait(state)
                .unwrap_or_else(|e| e.into_inner());
        }
        state.owner = Owner::Dictation;
        Lease(self)
    }

    pub fn try_background(&self) -> Option<Lease<'_>> {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.dictation_reserved || state.owner != Owner::Idle {
            return None;
        }
        state.owner = Owner::Background;
        Some(Lease(self))
    }

    pub fn cancel_dictation(&self, cancel: impl FnOnce()) {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.owner == Owner::Dictation {
            cancel();
        }
    }

    pub fn busy(&self) -> bool {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.dictation_reserved || state.owner != Owner::Idle
    }
}

impl Drop for Lease<'_> {
    fn drop(&mut self) {
        self.0.state.lock().unwrap_or_else(|e| e.into_inner()).owner = Owner::Idle;
        self.0.available.notify_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    #[test]
    fn dictation_has_priority_and_cancel_does_not_affect_meeting() {
        let scheduler = AsrScheduler::default();
        let epoch = AtomicU64::new(0);
        let meeting = scheduler.try_background().unwrap();
        scheduler.reserve_dictation();
        scheduler.cancel_dictation(|| panic!("must not cancel meeting"));
        drop(meeting);
        assert!(scheduler.try_background().is_none());
        let dictation = scheduler.dictation();
        scheduler.cancel_dictation(|| {
            epoch.fetch_add(1, Ordering::SeqCst);
        });
        assert_eq!(epoch.load(Ordering::SeqCst), 1);
        scheduler.finish_dictation();
        assert!(scheduler.try_background().is_none());
        drop(dictation);
        assert!(scheduler.try_background().is_some());
    }
}
