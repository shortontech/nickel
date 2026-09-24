//! Owns the Windows UWP shell STA inside the Nickel process.

use std::{
    sync::mpsc::{self, Receiver, TryRecvError},
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};
use windows::Win32::UI::WindowsAndMessaging::GetShellWindow;

pub(crate) struct UwuSupervisor {
    host: Option<UwuHost>,
    starting: Option<Receiver<Result<Option<UwuHost>, String>>>,
    retry_at: Option<Instant>,
}

impl UwuSupervisor {
    pub(crate) fn start() -> Self {
        let (host, retry_at) = match UwuHost::start(true) {
            Ok(Some(host)) => (Some(host), None),
            Ok(None) => (None, Some(Instant::now() + Duration::from_secs(5))),
            Err(error) => {
                tracing::warn!(%error, "embedded Windows UWP shell host unavailable");
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
        if self.host.as_ref().is_some_and(UwuHost::exited) {
            tracing::warn!("embedded Windows UWP shell host exited; retrying");
            self.host = None;
            self.retry_at = Some(Instant::now() + Duration::from_secs(5));
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
                    tracing::warn!(%error, "embedded Windows UWP shell host restart failed");
                    self.starting = None;
                    self.retry_at = Some(Instant::now() + Duration::from_secs(30));
                }
                Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Disconnected) => {
                    tracing::warn!("embedded Windows UWP shell host restart task exited");
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
                .name("nickel-uwu-restart".into())
                .spawn(move || {
                    let _ = sender.send(UwuHost::start(false));
                }) {
                Ok(_) => {
                    self.starting = Some(receiver);
                    self.retry_at = None;
                }
                Err(error) => {
                    tracing::warn!(%error, "embedded Windows UWP shell host restart task failed");
                    self.retry_at = Some(Instant::now() + Duration::from_secs(30));
                }
            }
        }
    }
}

pub(crate) struct UwuHost {
    thread: JoinHandle<()>,
}

impl UwuHost {
    fn start(configure_process_dpi: bool) -> Result<Option<Self>, String> {
        // Explorer and other registered shells already provide this controller.
        if !unsafe { GetShellWindow() }.is_invalid() {
            return Ok(None);
        }
        let (ready_tx, ready_rx) = mpsc::sync_channel(1);
        let thread = thread::Builder::new()
            .name("nickel-uwu-sta".into())
            .spawn(move || {
                let status = nickel_uwu::run_embedded_host(ready_tx, configure_process_dpi);
                if status != std::process::ExitCode::SUCCESS {
                    tracing::warn!(?status, "embedded Windows UWP shell host failed");
                }
            })
            .map_err(|error| format!("start embedded UWP shell host: {error}"))?;
        match ready_rx.recv_timeout(Duration::from_secs(30)) {
            Ok(Ok(())) => Ok(Some(Self { thread })),
            Ok(Err(error)) => Err(error),
            Err(error) => Err(format!("waiting for embedded UWP shell host: {error}")),
        }
    }

    fn exited(&self) -> bool {
        self.thread.is_finished()
    }
}
