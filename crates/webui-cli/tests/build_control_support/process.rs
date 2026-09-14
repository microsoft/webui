// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use std::fs;
use std::io::{BufRead, BufReader, Read};
use std::net::{SocketAddr, TcpListener};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, Context, Result};

const DEADLINE: Duration = Duration::from_secs(15);
static LOG_SEQUENCE: AtomicUsize = AtomicUsize::new(0);

pub struct Server {
    child: Child,
    stdin: Option<ChildStdin>,
    lines: mpsc::Receiver<String>,
    readers: Vec<JoinHandle<()>>,
    stdout: Arc<Mutex<String>>,
    stderr: Arc<Mutex<String>>,
    address: Option<SocketAddr>,
    out_dir: PathBuf,
    log_path: PathBuf,
}

impl Server {
    #[cfg(any(unix, windows))]
    pub fn pid(&self) -> u32 {
        self.child.id()
    }

    pub fn spawn(mut command: Command) -> Result<Self> {
        let mut args = command.get_args();
        let out_dir = args
            .find(|arg| *arg == "--servedir")
            .and_then(|_| args.next())
            .map(PathBuf::from)
            .context("fixture servedir argument")?;
        let log_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("target")
            .join("test-results")
            .join("client-builder");
        fs::create_dir_all(&log_dir)?;
        let test_name = thread::current()
            .name()
            .unwrap_or("client-builder")
            .replace("::", "-");
        let log_path = log_dir.join(format!(
            "{test_name}-{}-{}",
            std::process::id(),
            LOG_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        let child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .context("spawn native webui binary")?;
        let (send, lines) = mpsc::channel();
        let mut server = Self {
            child,
            stdin: None,
            lines,
            readers: Vec::with_capacity(2),
            stdout: Arc::new(Mutex::new(String::new())),
            stderr: Arc::new(Mutex::new(String::new())),
            address: None,
            out_dir,
            log_path,
        };
        server.stdin = server.child.stdin.take();
        let stdout = server.child.stdout.take().context("piped stdout")?;
        let logs = Arc::clone(&server.stdout);
        server
            .readers
            .push(thread::spawn(move || read_pipe(stdout, logs, None)));
        let stderr = server.child.stderr.take().context("piped stderr")?;
        let logs = Arc::clone(&server.stderr);
        server.readers.push(thread::spawn(move || {
            read_pipe(stderr, logs, Some(send));
        }));
        Ok(server)
    }

    pub fn listening(&mut self) -> Result<SocketAddr> {
        let deadline = Instant::now() + DEADLINE;
        loop {
            let line = self
                .lines
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                .with_context(|| format!("waiting for Local URL; stderr:\n{}", self.logs()))?;
            let Some((_, url)) = line.split_once("Local: ") else {
                continue;
            };
            let address: SocketAddr = url
                .trim()
                .strip_prefix("http://")
                .context("Local URL must be HTTP")?
                .trim_end_matches('/')
                .parse()
                .with_context(|| format!("Local socket address: {url}"))?;
            assert!(address.ip().is_loopback());
            assert_ne!(address.port(), 0, "advertise the actual bound port");
            self.address = Some(address);
            return Ok(address);
        }
    }

    pub fn close_stdin(&mut self) {
        self.stdin.take();
    }

    pub fn assert_alive(&mut self, duration: Duration) -> Result<()> {
        let deadline = Instant::now() + duration;
        loop {
            assert!(
                self.child.try_wait()?.is_none(),
                "server exited unexpectedly; stderr:\n{}",
                self.logs()
            );
            if Instant::now() >= deadline {
                return Ok(());
            }
            thread::sleep(Duration::from_millis(10));
        }
    }

    pub fn no_listening_url(&self) {
        assert!(
            !self.logs().contains("Local: "),
            "invalid startup must not advertise a listener: {}",
            self.logs()
        );
    }

    pub fn wait_exit(&mut self, success: bool) -> Result<ExitStatus> {
        let deadline = Instant::now() + DEADLINE;
        loop {
            if let Some(status) = self.child.try_wait()? {
                self.assert_released()?;
                self.finish_readers()?;
                self.save_logs()?;
                assert_eq!(
                    status.success(),
                    success,
                    "{status}; stderr:\n{}",
                    self.logs()
                );
                return Ok(status);
            }
            if Instant::now() >= deadline {
                bail!("webui did not exit; stderr:\n{}", self.logs());
            }
            thread::sleep(Duration::from_millis(10));
        }
    }

    pub fn shutdown(&mut self) -> Result<()> {
        self.close_stdin();
        self.wait_exit(true)?;
        Ok(())
    }

    pub fn failed(&mut self, diagnostic: &str) -> Result<()> {
        self.wait_exit(false)?;
        let logs = self.logs();
        let plain = logs.to_ascii_lowercase();
        let matches_diagnostic =
            plain.contains(diagnostic) || (diagnostic == "exit" && plain.contains("output closed"));
        assert!(
            matches_diagnostic,
            "missing {diagnostic:?} diagnostic: {logs}"
        );
        assert!(
            [
                "help:",
                "; check ",
                "; fix ",
                "; install ",
                "; add ",
                "; restart ",
                "then restart",
                "select --client-entry",
                "adjust --client-build-timeout-ms",
            ]
            .iter()
            .any(|remedy| plain.contains(remedy)),
            "failure must give actionable help: {logs}"
        );
        assert!(!logs.contains('\u{1b}'), "NO_COLOR diagnostics: {logs}");
        Ok(())
    }

    pub fn logs(&self) -> String {
        self.stderr
            .lock()
            .map(|logs| logs.clone())
            .unwrap_or_default()
    }

    pub fn stdout(&self) -> String {
        self.stdout
            .lock()
            .map(|logs| logs.clone())
            .unwrap_or_default()
    }

    fn assert_released(&self) -> Result<()> {
        if let Some(address) = self.address {
            let _listener = TcpListener::bind(address).context("reuse stopped server port")?;
        }
        let pid_file = self.out_dir.join("worker.pid");
        if pid_file.exists() {
            let pid = fs::read_to_string(pid_file)?;
            let status = Command::new("node")
                .args([
                    "-e",
                    "try { process.kill(Number(process.argv[1]), 0); process.exit(1); }\
                     catch (e) { process.exit(e.code === 'ESRCH' ? 0 : 2); }",
                    pid.trim(),
                ])
                .status()
                .context("verify builder worker was reaped")?;
            assert!(status.success(), "worker {pid} survived server exit");
        }
        Ok(())
    }

    fn finish_readers(&mut self) -> Result<()> {
        let deadline = Instant::now() + Duration::from_secs(3);
        while self.readers.iter().any(|reader| !reader.is_finished()) {
            anyhow::ensure!(
                Instant::now() < deadline,
                "server pipes remained open after exit; stderr:\n{}",
                self.logs()
            );
            thread::sleep(Duration::from_millis(10));
        }
        for reader in self.readers.drain(..) {
            reader.join().map_err(|_| anyhow!("pipe reader panicked"))?;
        }
        Ok(())
    }

    fn save_logs(&self) -> Result<()> {
        fs::write(self.log_path.with_extension("stderr.log"), self.logs())?;
        fs::write(self.log_path.with_extension("stdout.log"), self.stdout())?;
        let calls = self.out_dir.join("calls.ndjson");
        if calls.exists() {
            fs::copy(calls, self.log_path.with_extension("calls.ndjson"))?;
        }
        Ok(())
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.close_stdin();
        let deadline = Instant::now() + Duration::from_secs(3);
        while matches!(self.child.try_wait(), Ok(None)) && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(10));
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
        // A failing lifecycle test still owns, and must clean up, its fixture worker.
        if let Ok(pid) = fs::read_to_string(self.out_dir.join("worker.pid")) {
            if pid.trim().parse::<u32>().is_ok() {
                let _ = Command::new("node")
                    .args([
                        "-e",
                        "try { process.kill(Number(process.argv[1])); } catch {}",
                        pid.trim(),
                    ])
                    .status();
            }
        }
        let _ = self.finish_readers();
        let _ = self.save_logs();
    }
}

fn read_pipe(pipe: impl Read, logs: Arc<Mutex<String>>, sender: Option<mpsc::Sender<String>>) {
    for line in BufReader::new(pipe).lines() {
        let Ok(line) = line else { break };
        let Ok(mut logs) = logs.lock() else { break };
        logs.push_str(&line);
        logs.push('\n');
        if let Some(sender) = &sender {
            let _ = sender.send(line);
        }
    }
}
