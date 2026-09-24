//! Owns the Windows UWP shell helper for the lifetime of Nickel.

use std::{
    io::{BufRead, BufReader},
    process::{Child, Command, Stdio},
    sync::mpsc::{self, Receiver, TryRecvError},
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};
use windows::Win32::UI::WindowsAndMessaging::GetShellWindow;

/// Keeps the helper alive without waiting for a restart on Nickel's UI thread.
pub(crate) struct UwuSupervisor {
    host: Option<UwuHost>,
    starting: Option<Receiver<Result<Option<UwuHost>, String>>>,
    retry_at: Option<Instant>,
}

impl UwuSupervisor {
    pub(crate) fn start() -> Self {
        let (host, retry_at) = match UwuHost::start() {
            Ok(Some(host)) => (Some(host), None),
            Ok(None) => (None, Some(Instant::now() + Duration::from_secs(5))),
            Err(error) => {
                tracing::warn!(%error, "Windows UWP shell host unavailable");
                (None, Some(Instant::now() + Duration::from_secs(30)))
            }
        };
        Self {
            host,
            starting: None,
            retry_at,
        }
    }

    pub(crate) fn poll(&mut self) {
        if let Some(host) = self.host.as_mut() {
            match host.exited() {
                Ok(false) => {}
                Ok(true) => {
                    tracing::warn!("Windows UWP shell host exited; retrying");
                    self.host = None;
                    self.retry_at = Some(Instant::now() + Duration::from_secs(5));
                }
                Err(error) => {
                    tracing::warn!(%error, "Windows UWP shell host status unavailable");
                    self.host = None;
                    self.retry_at = Some(Instant::now() + Duration::from_secs(5));
                }
            }
        }
        if let Some(starting) = self.starting.as_ref() {
            match starting.try_recv() {
                Ok(Ok(Some(host))) => {
                    self.host = Some(host);
                    self.starting = None;
                    self.retry_at = None;
                }
                Ok(Ok(None)) => {
                    self.starting = None;
                    self.retry_at = Some(Instant::now() + Duration::from_secs(5));
                }
                Ok(Err(error)) => {
                    tracing::warn!(%error, "Windows UWP shell host restart failed");
                    self.starting = None;
                    self.retry_at = Some(Instant::now() + Duration::from_secs(30));
                }
                Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Disconnected) => {
                    tracing::warn!("Windows UWP shell host restart task exited");
                    self.starting = None;
                    self.retry_at = Some(Instant::now() + Duration::from_secs(30));
                }
            }
        }
        if self.host.is_none()
            && self.starting.is_none()
            && self
                .retry_at
                .is_some_and(|deadline| Instant::now() >= deadline)
        {
            let (sender, receiver) = mpsc::sync_channel(1);
            match thread::Builder::new()
                .name("nickel-uwu-start".into())
                .spawn(move || {
                    let _ = sender.send(UwuHost::start());
                }) {
                Ok(_) => {
                    self.starting = Some(receiver);
                    self.retry_at = None;
                }
                Err(error) => {
                    tracing::warn!(%error, "Windows UWP shell host restart task failed");
                    self.retry_at = Some(Instant::now() + Duration::from_secs(30));
                }
            }
        }
    }
}

pub(crate) struct UwuHost {
    child: Child,
    output: Option<JoinHandle<()>>,
    errors: Option<JoinHandle<()>>,
}

impl UwuHost {
    pub(crate) fn start() -> Result<Option<Self>, String> {
        // The immersive controller is only needed when Nickel owns the shell
        // session. Explorer and other registered shells already provide it.
        if !unsafe { GetShellWindow() }.is_invalid() {
            return Ok(None);
        }
        let executable = std::env::current_exe().map_err(|error| error.to_string())?;
        let mut child = Command::new(executable)
            .arg("--nickel-uwu-host")
            .arg(std::process::id().to_string())
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| format!("start UWP shell host: {error}"))?;
        let stdout = child.stdout.take().expect("piped UWP shell host stdout");
        let stderr = child.stderr.take().expect("piped UWP shell host stderr");
        let (ready_tx, ready_rx) = mpsc::sync_channel(1);
        let output = thread::Builder::new()
            .name("nickel-uwu-output".into())
            .spawn(move || {
                let mut ready = Some(ready_tx);
                for line in BufReader::new(stdout).lines() {
                    match line {
                        Ok(line) => {
                            if line == "phase=host-message-loop" {
                                if let Some(sender) = ready.take() {
                                    let _ = sender.send(Ok(()));
                                }
                            }
                            if line == "phase=host-message-loop"
                                || line.starts_with("phase=auto-present result=ready")
                            {
                                tracing::info!(message = %line, "UWP shell host");
                            } else {
                                tracing::debug!(message = %line, "UWP shell host");
                            }
                        }
                        Err(error) => {
                            if let Some(sender) = ready.take() {
                                let _ = sender.send(Err(error.to_string()));
                            }
                            break;
                        }
                    }
                }
                if let Some(sender) = ready {
                    let _ = sender.send(Err("UWP shell host exited before ready".into()));
                }
            })
            .map_err(|error| {
                let _ = child.kill();
                let _ = child.wait();
                format!("read UWP shell host output: {error}")
            })?;
        let errors = thread::Builder::new()
            .name("nickel-uwu-errors".into())
            .spawn(move || {
                for line in BufReader::new(stderr).lines() {
                    match line {
                        Ok(line) => tracing::warn!(message = %line, "UWP shell host"),
                        Err(error) => {
                            tracing::warn!(%error, "reading UWP shell host errors failed");
                            break;
                        }
                    }
                }
            });
        let errors = match errors {
            Ok(errors) => errors,
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = output.join();
                return Err(format!("read UWP shell host errors: {error}"));
            }
        };
        let host = Self {
            child,
            output: Some(output),
            errors: Some(errors),
        };
        match ready_rx.recv_timeout(Duration::from_secs(30)) {
            Ok(Ok(())) => Ok(Some(host)),
            Ok(Err(error)) => Err(error),
            Err(error) => Err(format!("waiting for UWP shell host: {error}")),
        }
    }

    pub(crate) fn exited(&mut self) -> Result<bool, String> {
        self.child
            .try_wait()
            .map(|status| status.is_some())
            .map_err(|error| error.to_string())
    }
}

impl Drop for UwuHost {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        // The pipes close when the child exits, so both readers can finish.
        if let Some(output) = self.output.take() {
            let _ = output.join();
        }
        if let Some(errors) = self.errors.take() {
            let _ = errors.join();
        }
    }
}
