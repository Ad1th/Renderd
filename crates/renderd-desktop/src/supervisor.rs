//! Runs one child process and keeps the session status current.

use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Read};
use std::process::Child;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread::{self, JoinHandle};

use crate::event::{host_event, viewer_event};
use crate::logline::{Level, LogLine};
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

impl Supervisor {
    /// Switches between sharing and viewing.
    ///
    /// Only possible while nothing is running. Returns whether the role is
    /// now `role`.
    pub fn set_role(&self, role: Role) -> bool {
        let status = {
            let mut shared = self.lock();
            if shared.status.phase.is_running() {
                return shared.status.role == role;
            }
            shared.status = Status::new(role);
            shared.status.clone()
        };
        (self.listener)(Update::Status(status));
        true
    }
}

impl Supervisor {
    /// Takes one line of child output from run number `run`.
    ///
    /// The line is kept for the details view, and if it changes the session
    /// the new status is sent on.
    fn handle_line(&self, run: u64, text: &str) {
        if text.trim().is_empty() {
            return;
        }
        let line = LogLine::parse(text);
        let changed = {
            let mut shared = self.lock();
            if shared.run != run {
                return;
            }
            if line.level >= Level::Error {
                shared.last_error = Some(line.message.clone());
            }
            if shared.logs.len() == LOG_CAPACITY {
                shared.logs.pop_front();
            }
            shared.logs.push_back(line.clone());

            let event = match shared.status.role {
                Role::Host => host_event(&line),
                Role::Viewer => viewer_event(&line),
            };
            event.and_then(|event| {
                let before = shared.status.clone();
                shared.status.apply(&event);
                (shared.status != before).then(|| shared.status.clone())
            })
        };
        (self.listener)(Update::Log(line));
        if let Some(status) = changed {
            (self.listener)(Update::Status(status));
        }
    }
}

impl Supervisor {
    /// Reads `pipe` line by line on its own thread until the child closes it.
    fn pump(&self, run: u64, pipe: impl Read + Send + 'static) -> JoinHandle<()> {
        let supervisor = self.clone();
        thread::spawn(move || {
            for line in BufReader::new(pipe).lines() {
                match line {
                    Ok(text) => supervisor.handle_line(run, &text),
                    Err(_) => break,
                }
            }
        })
    }
}
