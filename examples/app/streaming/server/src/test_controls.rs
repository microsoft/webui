// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Explicit release gates used only when the example server is started with
//! `--test-controls`.

use std::collections::HashMap;
use std::sync::{Arc, Condvar, Mutex, MutexGuard};

use tokio::sync::watch;

const MAX_SESSION_ID_BYTES: usize = 64;
const MAX_SESSIONS: usize = 128;

#[derive(Default)]
pub(crate) struct TestControls {
    sessions: Mutex<HashMap<String, Arc<TestSession>>>,
}

impl TestControls {
    pub(crate) fn session(&self, id: &str) -> Option<Arc<TestSession>> {
        if id.is_empty()
            || id.len() > MAX_SESSION_ID_BYTES
            || !id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        {
            return None;
        }
        let mut sessions = lock_unpoisoned(&self.sessions);
        if let Some(session) = sessions.get(id) {
            return Some(Arc::clone(session));
        }
        if sessions.len() == MAX_SESSIONS {
            return None;
        }
        let session = Arc::new(TestSession::new());
        sessions.insert(id.to_owned(), Arc::clone(&session));
        Some(session)
    }

    pub(crate) fn existing_session(&self, id: &str) -> Option<Arc<TestSession>> {
        lock_unpoisoned(&self.sessions).get(id).cloned()
    }
}

pub(crate) struct TestSession {
    released_feed_gaps: Mutex<usize>,
    feed_ready: Condvar,
    weather_released: watch::Sender<bool>,
}

impl TestSession {
    fn new() -> Self {
        let (weather_released, _) = watch::channel(false);
        Self {
            released_feed_gaps: Mutex::new(0),
            feed_ready: Condvar::new(),
            weather_released,
        }
    }

    pub(crate) fn wait_for_feed_gap(&self, gap: usize) {
        let required = gap.saturating_add(1);
        let mut released = lock_unpoisoned(&self.released_feed_gaps);
        while *released < required {
            released = wait_unpoisoned(&self.feed_ready, released);
        }
    }

    pub(crate) async fn wait_for_weather(&self) {
        let mut released = self.weather_released.subscribe();
        while !*released.borrow_and_update() {
            if released.changed().await.is_err() {
                return;
            }
        }
    }

    pub(crate) fn release_next_feed_gap(&self) {
        let mut released = lock_unpoisoned(&self.released_feed_gaps);
        *released = released.saturating_add(1);
        self.feed_ready.notify_all();
    }

    pub(crate) fn release_weather(&self) {
        self.weather_released.send_replace(true);
    }

    pub(crate) fn release_all(&self) {
        let mut released = lock_unpoisoned(&self.released_feed_gaps);
        *released = usize::MAX;
        self.feed_ready.notify_all();
        drop(released);
        self.release_weather();
    }
}

fn lock_unpoisoned<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn wait_unpoisoned<'a, T>(condvar: &Condvar, guard: MutexGuard<'a, T>) -> MutexGuard<'a, T> {
    condvar
        .wait(guard)
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use super::TestControls;

    #[test]
    fn sessions_are_bounded_and_identifiers_are_safe() {
        let controls = TestControls::default();
        assert!(controls.session("case-1").is_some());
        assert!(controls.session("../case").is_none());
        assert!(controls.session("").is_none());
    }

    #[test]
    fn feed_releases_are_counted() {
        let controls = TestControls::default();
        let session = controls
            .session("feed")
            .unwrap_or_else(|| panic!("valid session"));
        session.release_next_feed_gap();
        session.wait_for_feed_gap(0);
    }
}
