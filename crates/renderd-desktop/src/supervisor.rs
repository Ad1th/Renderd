//! Runs one child process and keeps the session status current.

use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Read};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crate::event::{host_event, viewer_event};
use crate::logline::{Level, LogLine};
use crate::problem::{Problem, ProblemKind};
use crate::role::Role;
use crate::status::Status;

/// How long a child gets to shut down by itself before it is killed.
pub const STOP_GRACE: Duration = Duration::from_secs(3);

/// How often the child is checked for having exited.
const EXIT_POLL: Duration = Duration::from_millis(40);

/// How long the readers get to deliver the last lines after the child exits.
const DRAIN_GRACE: Duration = Duration::from_millis(300);

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
        let changed = event.and_then(|event| {
            let before = shared.status.clone();
            shared.status.apply(&event);
            (shared.status != before).then(|| shared.status.clone())
        });
        // Listeners may call back into the supervisor, so let go of the lock first.
        drop(shared);

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
    /// The child is polled rather than inferred from its pipes closing: a pipe
    /// stays open for as long as any process holds its write end, and that is
    /// not always only the child. Once it has exited the readers get a moment
    /// to hand over the last lines, so the exit is judged with all of them.
    fn reap(&self, run: u64, readers: Vec<JoinHandle<()>>) {
        let supervisor = self.clone();
        thread::spawn(move || {
            let Some(success) = supervisor.wait_for_exit(run) else {
                return;
            };
            let deadline = Instant::now() + DRAIN_GRACE;
            while readers.iter().any(|reader| !reader.is_finished()) && Instant::now() < deadline {
                thread::sleep(Duration::from_millis(5));
            }

            let mut shared = supervisor.lock();
            if shared.run != run {
                return;
            }
            shared.child = None;
            let last_error = shared.last_error.take();
            shared.status.exited(success, last_error.as_deref());
            let status = shared.status.clone();
            drop(shared);
            (supervisor.listener)(Update::Status(status));
        });
    }

    /// Blocks until the child of run number `run` has exited.
    ///
    /// Returns whether it exited successfully, or `None` if a newer run has
    /// started in the meantime.
    fn wait_for_exit(&self, run: u64) -> Option<bool> {
        loop {
            let exit = {
                let mut shared = self.lock();
                if shared.run != run {
                    return None;
                }
                shared.child.as_mut().map(Child::try_wait)
            };
            match exit {
                Some(Ok(Some(exit))) => return Some(exit.success()),
                Some(Ok(None)) => thread::sleep(EXIT_POLL),
                Some(Err(_)) | None => return Some(false),
            }
        }
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

/// Asks the child to shut down the way Ctrl+C would.
///
/// The host unregisters from mDNS and closes its socket on SIGINT, so it gets
/// the chance to do that before anything harsher.
#[cfg(unix)]
fn interrupt(child: &mut Child) {
    use nix::sys::signal::{kill, Signal};
    use nix::unistd::Pid;

    let sent =
        i32::try_from(child.id()).is_ok_and(|pid| kill(Pid::from_raw(pid), Signal::SIGINT).is_ok());
    if !sent {
        let _ = child.kill();
    }
}

/// Windows has no SIGINT for a process without a console, so end it directly.
#[cfg(not(unix))]
fn interrupt(child: &mut Child) {
    let _ = child.kill();
}

impl Supervisor {
    /// Asks the running session to stop.
    ///
    /// The child is interrupted first. If it is still alive after
    /// [`STOP_GRACE`] it is killed.
    pub fn stop(&self) {
        let (status, run) = {
            let mut shared = self.lock();
            if !shared.status.phase.is_running() {
                return;
            }
            shared.status.begin_stop();
            if let Some(child) = shared.child.as_mut() {
                interrupt(child);
            }
            (shared.status.clone(), shared.run)
        };
        (self.listener)(Update::Status(status));

        let supervisor = self.clone();
        thread::spawn(move || {
            thread::sleep(STOP_GRACE);
            let mut shared = supervisor.lock();
            if shared.run == run {
                if let Some(child) = shared.child.as_mut() {
                    let _ = child.kill();
                }
            }
        });
    }
}

impl Supervisor {
    /// Ends the child right now, without waiting. For when the app is quitting.
    pub fn shutdown(&self) {
        let mut shared = self.lock();
        shared.status.begin_stop();
        if let Some(child) = shared.child.as_mut() {
            interrupt(child);
            thread::sleep(Duration::from_millis(150));
            let _ = child.kill();
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::status::Phase;

    /// Runs a shell script as if it were the host or viewer.
    fn run_script(supervisor: &Supervisor, script: &str) -> Status {
        supervisor.start(
            Path::new("/bin/sh"),
            &["-c".to_string(), script.to_string()],
        )
    }

    /// Polls until the status satisfies `done`, failing after five seconds.
    fn wait_for(supervisor: &Supervisor, done: impl Fn(&Status) -> bool) -> Status {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let status = supervisor.status();
            if done(&status) {
                return status;
            }
            assert!(Instant::now() < deadline, "timed out in {:?}", status.phase);
            thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn a_clean_exit_ends_stopped() {
        let supervisor = Supervisor::new(Role::Host, |_| {});
        let status = run_script(&supervisor, "exit 0");
        assert_eq!(status.phase, Phase::Starting);
        let status = wait_for(&supervisor, |s| !s.phase.is_running());
        assert_eq!(status.phase, Phase::Stopped);
    }

    #[test]
    fn a_failing_child_is_explained_by_its_last_error() {
        let supervisor = Supervisor::new(Role::Host, |_| {});
        run_script(
            &supervisor,
            "echo 'Error: Address already in use (os error 48)' >&2; exit 1",
        );
        let status = wait_for(&supervisor, |s| !s.phase.is_running());
        assert_eq!(status.phase, Phase::Failed);
        assert_eq!(status.problem.unwrap().kind, ProblemKind::PortInUse);
    }

    #[test]
    fn a_missing_program_fails_at_once() {
        let supervisor = Supervisor::new(Role::Viewer, |_| {});
        let status = supervisor.start(Path::new("/nonexistent/renderd-viewer"), &[]);
        assert_eq!(status.phase, Phase::Failed);
        assert_eq!(status.problem.unwrap().kind, ProblemKind::MissingBinary);
    }

    #[test]
    fn json_output_moves_the_status() {
        let supervisor = Supervisor::new(Role::Host, |_| {});
        let listening = r#"{"level":"INFO","fields":{"message":"QUIC server endpoint listening for incoming viewer connections","listen_addr":"0.0.0.0:4433"},"target":"t"}"#;
        run_script(&supervisor, &format!("echo '{listening}' >&2; sleep 30"));
        let status = wait_for(&supervisor, |s| s.phase == Phase::Waiting);
        assert_eq!(status.phase, Phase::Waiting);
        supervisor.shutdown();
    }

    #[test]
    fn stop_ends_a_long_running_child() {
        let supervisor = Supervisor::new(Role::Host, |_| {});
        run_script(&supervisor, "sleep 30");
        supervisor.stop();
        assert_eq!(supervisor.status().phase, Phase::Stopping);
        let status = wait_for(&supervisor, |s| !s.phase.is_running());
        assert_eq!(status.phase, Phase::Stopped);
        assert!(status.problem.is_none());
    }

    #[test]
    fn a_second_start_is_ignored_while_running() {
        let supervisor = Supervisor::new(Role::Host, |_| {});
        run_script(&supervisor, "sleep 30");
        let again = run_script(&supervisor, "exit 1");
        assert_eq!(again.phase, Phase::Starting);
        supervisor.stop();
        let status = wait_for(&supervisor, |s| !s.phase.is_running());
        assert_eq!(status.phase, Phase::Stopped);
    }

    #[test]
    fn the_role_cannot_change_while_running() {
        let supervisor = Supervisor::new(Role::Host, |_| {});
        assert!(supervisor.set_role(Role::Viewer));
        run_script(&supervisor, "sleep 30");
        assert!(!supervisor.set_role(Role::Host));
        assert_eq!(supervisor.status().role, Role::Viewer);
        supervisor.shutdown();
    }

    #[test]
    fn output_is_kept_as_log_lines() {
        let supervisor = Supervisor::new(Role::Host, |_| {});
        run_script(&supervisor, "echo one; echo two >&2");
        wait_for(&supervisor, |s| !s.phase.is_running());
        let mut messages: Vec<_> = supervisor.logs().into_iter().map(|l| l.message).collect();
        messages.sort();
        assert_eq!(messages, ["one", "two"]);
    }

    #[test]
    fn the_log_buffer_is_bounded() {
        let supervisor = Supervisor::new(Role::Host, |_| {});
        run_script(
            &supervisor,
            "i=0; while [ $i -lt 600 ]; do echo line $i; i=$((i+1)); done",
        );
        wait_for(&supervisor, |s| !s.phase.is_running());
        let logs = supervisor.logs();
        assert_eq!(logs.len(), LOG_CAPACITY);
        assert_eq!(logs.last().unwrap().message, "line 599");
    }

    #[test]
    fn listeners_hear_status_changes_and_logs() {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&seen);
        let supervisor = Supervisor::new(Role::Host, move |update| {
            sink.lock().unwrap().push(update);
        });
        run_script(&supervisor, "echo hello");
        wait_for(&supervisor, |s| !s.phase.is_running());
        // The final status is sent just after it becomes visible to status().
        thread::sleep(Duration::from_millis(50));
        let updates = seen.lock().unwrap().clone();
        assert!(matches!(updates.first(), Some(Update::Status(s)) if s.phase == Phase::Starting));
        assert!(updates
            .iter()
            .any(|u| matches!(u, Update::Log(line) if line.message == "hello")));
        assert!(matches!(updates.last(), Some(Update::Status(s)) if s.phase == Phase::Stopped));
    }

    #[test]
    fn starting_again_clears_old_logs() {
        let supervisor = Supervisor::new(Role::Host, |_| {});
        run_script(&supervisor, "echo first");
        wait_for(&supervisor, |s| !s.phase.is_running());
        run_script(&supervisor, "echo second");
        wait_for(&supervisor, |s| !s.phase.is_running());
        let messages: Vec<_> = supervisor.logs().into_iter().map(|l| l.message).collect();
        assert_eq!(messages, ["second"]);
    }
}
