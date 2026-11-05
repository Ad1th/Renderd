//! Runs one child process and keeps the session status current.

use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Read};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread::{self, JoinHandle};

use crate::event::{host_event, viewer_event};
use crate::logline::{Level, LogLine};
use crate::problem::{Problem, ProblemKind};
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

impl Supervisor {
    /// Waits for run number `run` to end and records how it went.
    ///
    /// Both pipes reach end of file when the child exits, so the readers are
    /// joined first. That also guarantees every line was seen before the exit
    /// is judged.
    fn reap(&self, run: u64, readers: Vec<JoinHandle<()>>) {
        let supervisor = self.clone();
        thread::spawn(move || {
            for reader in readers {
                let _ = reader.join();
            }
            let child = {
                let mut shared = supervisor.lock();
                if shared.run != run {
                    return;
                }
                shared.child.take()
            };
            let success = child
                .and_then(|mut child| child.wait().ok())
                .is_some_and(|exit| exit.success());
            let status = {
                let mut shared = supervisor.lock();
                if shared.run != run {
                    return;
                }
                let last_error = shared.last_error.take();
                shared.status.exited(success, last_error.as_deref());
                shared.status.clone()
            };
            (supervisor.listener)(Update::Status(status));
        });
    }
}

/// Builds the command for a child whose output the app reads.
fn piped_command(program: &Path, args: &[String]) -> Command {
    let mut command = Command::new(program);
    command
        .args(args)
        .env("NO_COLOR", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    hide_console(&mut command);
    command
}

/// Stops Windows from opening a console window for the child.
#[cfg(windows)]
fn hide_console(command: &mut Command) {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    command.creation_flags(CREATE_NO_WINDOW);
}

#[cfg(not(windows))]
const fn hide_console(_command: &mut Command) {}

impl Supervisor {
    /// Starts `program` with `args` and begins following its output.
    ///
    /// Does nothing if a session is already running. A program that cannot be
    /// started leaves the status failed with the reason.
    pub fn start(&self, program: &Path, args: &[String]) -> Status {
        let (status, started) = {
            let mut shared = self.lock();
            if shared.status.phase.is_running() {
                return shared.status.clone();
            }
            shared.run += 1;
            shared.logs.clear();
            shared.last_error = None;
            shared.status.begin();

            match piped_command(program, args).spawn() {
                Ok(mut child) => {
                    let pipes = (child.stdout.take(), child.stderr.take());
                    shared.child = Some(child);
                    (shared.status.clone(), Some((shared.run, pipes)))
                }
                Err(error) => {
                    let detail = format!("Could not start {}: {error}", program.display());
                    shared
                        .status
                        .fail(Problem::new(ProblemKind::MissingBinary, detail));
                    (shared.status.clone(), None)
                }
            }
        };
        (self.listener)(Update::Status(status.clone()));

        if let Some((run, (stdout, stderr))) = started {
            let mut readers = Vec::with_capacity(2);
            if let Some(pipe) = stdout {
                readers.push(self.pump(run, pipe));
            }
            if let Some(pipe) = stderr {
                readers.push(self.pump(run, pipe));
            }
            self.reap(run, readers);
        }
        status
    }
}
