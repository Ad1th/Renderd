//! Runs one child process and keeps the session status current.

use std::collections::VecDeque;
use std::process::Child;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use crate::logline::LogLine;
use crate::role::Role;
use crate::status::Status;

/// How many recent log lines are kept for the details view.
pub const LOG_CAPACITY: usize = 400;

/// Something the window should hear about.
#[derive(Debug, Clone, PartialEq)]
pub enum Update {
    /// The session status changed.
    Status(Status),
    /// The child wrote a log line.
    Log(LogLine),
}

/// Callback that receives every [`Update`].
pub type Listener = dyn Fn(Update) + Send + Sync;

/// State shared between the window's thread and the reader threads.
struct Shared {
    status: Status,
    logs: VecDeque<LogLine>,
    last_error: Option<String>,
    child: Option<Child>,
    /// Counts starts, so threads from an old run cannot touch a new one.
    run: u64,
}

/// Starts, watches and stops the host or viewer process.
#[derive(Clone)]
pub struct Supervisor {
    shared: Arc<Mutex<Shared>>,
    listener: Arc<Listener>,
}

impl Supervisor {
    /// Creates a supervisor with nothing running.
    pub fn new(role: Role, listener: impl Fn(Update) + Send + Sync + 'static) -> Self {
        Self {
            shared: Arc::new(Mutex::new(Shared {
                status: Status::new(role),
                logs: VecDeque::with_capacity(LOG_CAPACITY),
                last_error: None,
                child: None,
                run: 0,
            })),
            listener: Arc::new(listener),
        }
    }

    /// Locks the shared state. A panic on another thread must not take the
    /// whole app down with it, so a poisoned lock is used as it is.
    fn lock(&self) -> MutexGuard<'_, Shared> {
        self.shared.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl Supervisor {
    /// The current session status.
    #[must_use]
    pub fn status(&self) -> Status {
        self.lock().status.clone()
    }

    /// The most recent log lines, oldest first.
    #[must_use]
    pub fn logs(&self) -> Vec<LogLine> {
        self.lock().logs.iter().cloned().collect()
    }
}
