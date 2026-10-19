//! Running external programs without a shell, with a deterministic locale and a timeout.

use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant};

pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(20);

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CmdOutput {
    /// Exit code, `None` when the process was killed (signal or timeout).
    pub status: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub timed_out: bool,
}

impl CmdOutput {
    pub fn ok(stdout: impl Into<String>) -> Self {
        Self { status: Some(0), stdout: stdout.into(), ..Default::default() }
    }

    pub fn failed(code: i32, stderr: impl Into<String>) -> Self {
        Self { status: Some(code), stderr: stderr.into(), ..Default::default() }
    }

    pub fn success(&self) -> bool {
        self.status == Some(0)
    }

    /// stdout and stderr together, the way a user would see them in a terminal.
    pub fn combined(&self) -> String {
        match (self.stdout.trim().is_empty(), self.stderr.trim().is_empty()) {
            (false, false) => format!("{}\n{}", self.stdout.trim_end(), self.stderr.trim_end()),
            (false, true) => self.stdout.trim_end().to_string(),
            (true, _) => self.stderr.trim_end().to_string(),
        }
    }
}

/// Abstraction over process execution so probes and runtimes can be tested with canned output.
pub trait CommandRunner: Send + Sync {
    fn run_with_timeout(&self, program: &str, args: &[&str], timeout: Duration) -> std::io::Result<CmdOutput>;

    fn run(&self, program: &str, args: &[&str]) -> std::io::Result<CmdOutput> {
        self.run_with_timeout(program, args, DEFAULT_TIMEOUT)
    }

    /// Resolve a program in `PATH` (plus the sbin directories, which are often missing for users).
    fn which(&self, program: &str) -> Option<PathBuf>;

    /// Start a long-running program in its own process group, output appended to `log`.
    /// Returns the PID; the process outlives the call.
    fn spawn_detached(&self, program: &str, args: &[&str], log: &Path) -> std::io::Result<u32>;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct SystemRunner;

impl CommandRunner for SystemRunner {
    fn run_with_timeout(&self, program: &str, args: &[&str], timeout: Duration) -> std::io::Result<CmdOutput> {
        let mut child = Command::new(program)
            .args(args)
            .env("LC_ALL", "C")
            .env("LANG", "C")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;

        // Drain both pipes on threads so a chatty child cannot block on a full pipe.
        let mut out_pipe = child.stdout.take().expect("stdout piped");
        let mut err_pipe = child.stderr.take().expect("stderr piped");
        let out_thread = std::thread::spawn(move || {
            let mut buf = Vec::new();
            let _ = out_pipe.read_to_end(&mut buf);
            buf
        });
        let err_thread = std::thread::spawn(move || {
            let mut buf = Vec::new();
            let _ = err_pipe.read_to_end(&mut buf);
            buf
        });

        let start = Instant::now();
        let mut timed_out = false;
        let status = loop {
            if let Some(status) = child.try_wait()? {
                break Some(status);
            }
            if start.elapsed() >= timeout {
                timed_out = true;
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
            std::thread::sleep(Duration::from_millis(15));
        };

        let stdout = String::from_utf8_lossy(&out_thread.join().unwrap_or_default()).into_owned();
        let stderr = String::from_utf8_lossy(&err_thread.join().unwrap_or_default()).into_owned();
        Ok(CmdOutput { status: status.and_then(|s| s.code()), stdout, stderr, timed_out })
    }

    fn which(&self, program: &str) -> Option<PathBuf> {
        if program.contains('/') {
            let p = PathBuf::from(program);
            return is_executable(&p).then_some(p);
        }
        let path = std::env::var_os("PATH").unwrap_or_default();
        std::env::split_paths(&path)
            .chain(["/usr/sbin", "/sbin", "/usr/local/sbin"].iter().map(PathBuf::from))
            .map(|dir| dir.join(program))
            .find(|candidate| is_executable(candidate))
    }

    fn spawn_detached(&self, program: &str, args: &[&str], log: &Path) -> std::io::Result<u32> {
        use std::os::unix::process::CommandExt;
        if let Some(dir) = log.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let out = std::fs::OpenOptions::new().create(true).append(true).open(log)?;
        let err = out.try_clone()?;
        let mut child = Command::new(program)
            .args(args)
            .stdin(Stdio::null())
            .stdout(out)
            .stderr(err)
            .process_group(0)
            .spawn()?;
        let pid = child.id();
        // Reap it when it exits so it never lingers as a zombie of a long-lived GUI process.
        std::thread::spawn(move || {
            let _ = child.wait();
        });
        Ok(pid)
    }
}

fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path).map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0).unwrap_or(false)
}

/// Test double: answers commands from a table keyed by `"program arg1 arg2"`.
/// Responding twice to the same command line queues the answers; the last one repeats.
#[derive(Default)]
pub struct FakeRunner {
    responses: Mutex<HashMap<String, Vec<CmdOutput>>>,
    programs: Mutex<HashMap<String, PathBuf>>,
    calls: Mutex<Vec<String>>,
}

impl FakeRunner {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn respond(self, command_line: &str, output: CmdOutput) -> Self {
        self.responses.lock().unwrap().entry(command_line.to_string()).or_default().push(output);
        self
    }

    pub fn with_program(self, program: &str) -> Self {
        self.programs.lock().unwrap().insert(program.to_string(), PathBuf::from(format!("/usr/bin/{program}")));
        self
    }

    pub fn calls(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }
}

impl CommandRunner for FakeRunner {
    fn run_with_timeout(&self, program: &str, args: &[&str], _timeout: Duration) -> std::io::Result<CmdOutput> {
        let key = std::iter::once(program).chain(args.iter().copied()).collect::<Vec<_>>().join(" ");
        self.calls.lock().unwrap().push(key.clone());
        let mut responses = self.responses.lock().unwrap();
        match responses.get_mut(&key) {
            Some(queue) if queue.len() > 1 => Ok(queue.remove(0)),
            Some(queue) if !queue.is_empty() => Ok(queue[0].clone()),
            _ => Err(std::io::Error::new(std::io::ErrorKind::NotFound, format!("no fake response for `{key}`"))),
        }
    }

    fn which(&self, program: &str) -> Option<PathBuf> {
        self.programs.lock().unwrap().get(program).cloned()
    }

    fn spawn_detached(&self, program: &str, args: &[&str], _log: &Path) -> std::io::Result<u32> {
        let key = std::iter::once(program).chain(args.iter().copied()).collect::<Vec<_>>().join(" ");
        self.calls.lock().unwrap().push(format!("spawn {key}"));
        Ok(4242)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_runner_captures_output_and_exit_code() {
        let out = SystemRunner.run("sh", &["-c", "echo hi; echo err >&2; exit 3"]).unwrap();
        assert_eq!(out.status, Some(3));
        assert_eq!(out.stdout.trim(), "hi");
        assert_eq!(out.stderr.trim(), "err");
        assert_eq!(out.combined(), "hi\nerr");
    }

    #[test]
    fn system_runner_forces_c_locale() {
        let out = SystemRunner.run("sh", &["-c", "echo $LC_ALL"]).unwrap();
        assert_eq!(out.stdout.trim(), "C");
    }

    #[test]
    fn system_runner_kills_on_timeout() {
        let start = Instant::now();
        let out = SystemRunner.run_with_timeout("sleep", &["5"], Duration::from_millis(200)).unwrap();
        assert!(out.timed_out);
        assert_eq!(out.status, None);
        assert!(start.elapsed() < Duration::from_secs(3));
    }

    #[test]
    fn which_finds_sh() {
        assert!(SystemRunner.which("sh").is_some());
        assert!(SystemRunner.which("definitely-not-a-real-program-xyz").is_none());
    }
}
