// SPDX-License-Identifier: MIT
// SPDX-FileCopyrightText: Copyright (c) 2025 Asymptotic Inc.
// SPDX-FileCopyrightText: Copyright (c) 2025 Arun Raghavan

use std::{
    os::{
        fd::{AsRawFd, RawFd},
        unix::net::UnixStream,
    },
    path::PathBuf,
    sync::RwLock,
};

use pipewire_native_spa as spa;

use crate::{
    closure,
    core::{self, Core, WeakCore},
    debug, default_topic, keys, log, main_loop, new_refcounted,
    protocol::connection::{Connection, ConnectionEvents},
    proxy::{self, HasProxy},
    proxy_notify, refcounted, some_closure, trace, types, warn, Id,
};

default_topic!(log::topic::PROTOCOL);

fn get_runtime_dir() -> Option<String> {
    std::env::var("PIPEWIRE_RUNTIME_DIR")
        .or(std::env::var("XDG_RUNTIME_DIR"))
        .or(std::env::var("USERPROFILEDIR"))
        .ok()
}

fn get_system_dir() -> String {
    "/run/pipewire".to_owned()
}

refcounted! {
    pub(crate) struct Client {
        core: RwLock<Option<WeakCore>>,
        stream: RwLock<Option<UnixStream>>,
        connection: Connection,
        connected: RwLock<bool>,
        need_flush: RwLock<bool>,
        last_in_seq: RwLock<u32>,
        dispatch_limits: RwLock<Option<(usize, usize, usize)>>,
        source: RwLock<Option<main_loop::Source>>,
        hooks: RwLock<Option<spa::hook::HookId>>,
    }
}

impl Client {
    pub(crate) fn new() -> Self {
        debug!("Creating new client");
        let this = Self {
            inner: new_refcounted(InnerClient::new()),
        };

        let listener = this.inner.connection.add_listener(ConnectionEvents {
            destroy: some_closure!([this] {
                this.on_destroy();
            }),
            error: None,
            need_flush: some_closure!([this] {
                this.on_need_flush();
            }),
            start: None,
        });

        this.inner.hooks.write().unwrap().replace(listener);

        this
    }

    pub(crate) fn connection(&self) -> Connection {
        self.inner.connection.clone()
    }

    pub(crate) fn core(&self) -> Core {
        self.inner
            .core
            .read()
            .unwrap()
            .clone()
            .and_then(|w| w.upgrade())
            .expect("Client shoud have core initialised on creation")
    }

    pub(crate) fn set_core(&self, core: WeakCore) {
        self.inner.set_core(core);
    }

    pub(crate) fn connect(
        &self,
        props: Option<&spa::dict::Dict>,
        done_cb: Option<Box<dyn Fn(std::io::Result<()>)>>,
        timeout: Option<std::time::Duration>,
    ) -> std::io::Result<()> {
        // TODO: Implement PW_KEY_REMOTE_INTENTION != "generic" (i.e. screencast and internal remotes)
        self.connect_local_socket(
            props,
            done_cb,
            timeout.map(|duration| std::time::Instant::now() + duration),
        )
    }

    pub(crate) fn disconnect(&self) {
        let _ = self.inner.source.write().unwrap().take();
        let _ = self.inner.stream.write().unwrap().take();

        self.inner.connection.disconnect();
        *self.inner.connected.write().unwrap() = false;
        *self.inner.need_flush.write().unwrap() = false;

        *self.inner.last_in_seq.write().unwrap() = 0;
    }

    pub(crate) fn set_stream(&self, stream: UnixStream) -> std::io::Result<()> {
        debug!("Setting fd on connection: {stream:?}");

        let fd = stream.as_raw_fd();

        self.inner
            .connection
            .set_stream(stream.try_clone().expect("unix stream should be cloneable"));
        self.inner.stream.write().unwrap().replace(stream);
        *self.inner.connected.write().unwrap() = false;

        let main_loop = self.core().context().main_loop();

        let source = main_loop.add_io(
            fd,
            spa::flags::Io::all(),
            false,
            closure!([client <- self] fd, mask, {
                client.on_remote_data(fd, spa::flags::Io::from_bits_truncate(mask));
            }),
        );

        *self.inner.source.write().unwrap() = source;

        Ok(())
    }

    fn on_destroy(&self) {
        self.inner
            .connection
            .remove_listener(self.inner.hooks.read().unwrap().unwrap());
    }

    fn on_need_flush(&self) {
        *self.inner.need_flush.write().unwrap() = true;

        if let Some(source) = self.inner.source.write().unwrap().as_mut() {
            let main_loop = self.core().context().main_loop();
            let _ = main_loop.update_io(source, source.mask() | spa::flags::Io::OUT);
        }
    }

    pub(crate) fn set_dispatch_limits(&self, messages: usize, bytes: usize, frame_bytes: usize) {
        *self.inner.dispatch_limits.write().unwrap() = Some((messages, bytes, frame_bytes));
        self.inner.connection.set_frame_limit(frame_bytes);
    }

    fn on_remote_data(&self, _fd: RawFd, mask: spa::flags::Io) {
        trace!("on remote data: {mask:?}");

        if mask.intersects(spa::flags::Io::ERR | spa::flags::Io::HUP) {
            self.on_connection_error(
                std::io::Error::from(std::io::ErrorKind::BrokenPipe),
                "I/O error",
            );
            return;
        }

        let limits = *self.inner.dispatch_limits.read().unwrap();
        let mut input_yielded = false;
        if mask.contains(spa::flags::Io::IN)
            || (limits.is_some() && self.inner.connection.has_buffered_input())
        {
            let mut messages = 0;
            let mut bytes = 0;
            loop {
                if limits.is_some_and(|(max_messages, max_bytes, max_frame)| {
                    messages >= max_messages || bytes + max_frame > max_bytes
                }) {
                    input_yielded = true;
                    break;
                }
                let result = self.process_messages();
                if let Ok(consumed) = result.as_ref() {
                    messages += 1;
                    bytes += consumed;
                }
                if let Err(err) = result {
                    // We use EAGAIN to signify there are no more messages pending
                    if err.raw_os_error() == Some(libc::EAGAIN) {
                        break;
                    } else {
                        self.on_connection_error(err, "failed to read messages");
                        return;
                    }
                }
            }
        }

        if mask.contains(spa::flags::Io::OUT) || *self.inner.need_flush.read().unwrap() {
            *self.inner.need_flush.write().unwrap() = true;

            match self
                .inner
                .stream
                .read()
                .unwrap()
                .as_ref()
                .unwrap()
                .take_error()
            {
                Ok(None) => { /* all good, nothing to do */ }
                Ok(Some(err)) => {
                    self.on_connection_error(err, "connection error");
                    return;
                }
                Err(err) => {
                    self.on_connection_error(err, "getsockopt failed");
                    return;
                }
            }

            match self.inner.connection.flush() {
                Ok(_) => {
                    let main_loop = self.core().context().main_loop();
                    let mut source_ref = self.inner.source.write().unwrap();
                    let source = source_ref.as_mut().unwrap();
                    let next_mask = if input_yielded && self.inner.connection.has_buffered_input() {
                        source.mask() | spa::flags::Io::OUT
                    } else {
                        source.mask() & !spa::flags::Io::OUT
                    };
                    let _ = main_loop.update_io(source, next_mask);
                }
                Err(err) => {
                    if err.raw_os_error() != Some(libc::EAGAIN) {
                        self.on_connection_error(err, "flush failed");
                    }
                }
            }
        }
    }

    fn process_messages(&self) -> std::io::Result<usize> {
        let core = self.core();
        let header = self.inner.connection.next_message()?;
        let object_type = match core.find_proxy_type(header.id as Id) {
            Some(type_) => type_,
            None => {
                warn!(
                    "Got message id:{} opcode:{} seq:{}",
                    header.id, header.opcode, header.seq
                );
                if self.inner.dispatch_limits.read().unwrap().is_some() {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        "unknown guarded proxy",
                    ));
                }
                return Ok(0);
            }
        };

        match object_type {
            types::interface::CORE => {
                let proxy = core.find_proxy::<Core>(header.id).unwrap();
                super::marshal::core::Events::demarshal(&self.inner.connection, &header, proxy)?;
            }
            types::interface::CLIENT => {
                let proxy = core.find_proxy::<proxy::client::Client>(header.id).unwrap();
                super::marshal::client::Events::demarshal(&self.inner.connection, &header, proxy)?;
            }
            types::interface::DEVICE => {
                let proxy = core.find_proxy::<proxy::device::Device>(header.id).unwrap();
                super::marshal::device::Events::demarshal(&self.inner.connection, &header, proxy)?;
            }
            types::interface::FACTORY => {
                let proxy = core
                    .find_proxy::<proxy::factory::Factory>(header.id)
                    .unwrap();
                super::marshal::factory::Events::demarshal(&self.inner.connection, &header, proxy)?;
            }
            types::interface::LINK => {
                let proxy = core.find_proxy::<proxy::link::Link>(header.id).unwrap();
                super::marshal::link::Events::demarshal(&self.inner.connection, &header, proxy)?;
            }
            types::interface::METADATA => {
                let proxy = core
                    .find_proxy::<proxy::metadata::Metadata>(header.id)
                    .unwrap();
                super::marshal::metadata::Events::demarshal(
                    &self.inner.connection,
                    &header,
                    proxy,
                )?;
            }
            types::interface::MODULE => {
                let proxy = core.find_proxy::<proxy::module::Module>(header.id).unwrap();
                super::marshal::module::Events::demarshal(&self.inner.connection, &header, proxy)?;
            }
            types::interface::NODE => {
                let proxy = core.find_proxy::<proxy::node::Node>(header.id).unwrap();
                super::marshal::node::Events::demarshal(&self.inner.connection, &header, proxy)?;
            }
            types::interface::PORT => {
                let proxy = core.find_proxy::<proxy::port::Port>(header.id).unwrap();
                super::marshal::port::Events::demarshal(&self.inner.connection, &header, proxy)?;
            }
            types::interface::PROFILER => {
                let proxy = core
                    .find_proxy::<proxy::profiler::Profiler>(header.id)
                    .unwrap();
                super::marshal::profiler::Events::demarshal(
                    &self.inner.connection,
                    &header,
                    proxy,
                )?;
            }
            types::interface::REGISTRY => {
                let proxy = core
                    .find_proxy::<proxy::registry::Registry>(header.id)
                    .unwrap();
                super::marshal::registry::Events::demarshal(
                    &self.inner.connection,
                    &header,
                    proxy,
                )?;
            }
            _ => unreachable!(),
        }

        *self.inner.last_in_seq.write().unwrap() = header.seq;

        Ok(super::marshal::HEADER_LEN + header.size as usize)
    }

    fn on_connection_error(&self, err: std::io::Error, msg: &str) {
        warn!("Got connection error: {:?}", err);

        if let Some(source) = self.inner.source.write().unwrap().take() {
            let main_loop = self.core().context().main_loop();
            main_loop.destroy_source(source);
        }

        let core = &self.core();
        let seq = *self.inner.last_in_seq.read().unwrap();
        let res = err
            .raw_os_error()
            .unwrap_or(err.kind() as i32)
            .unsigned_abs();

        proxy_notify!(core, error, seq, res, msg);
    }

    fn connect_local_socket(
        &self,
        props: Option<&spa::dict::Dict>,
        done_cb: Option<Box<dyn Fn(std::io::Result<()>)>>,
        deadline: Option<std::time::Instant>,
    ) -> std::io::Result<()> {
        let manager = props.and_then(|p| p.lookup(keys::REMOTE_INTENTION)) == Some("manager");
        let mut remote_name = core::get_remote(props);

        // TODO: remote can be a list of remotes

        if manager && !remote_name.ends_with("-manager") {
            remote_name = format!("{remote_name}-manager");
        }

        if remote_name.starts_with("/") || remote_name.starts_with("@") {
            // Absolute path
            self.try_connect_local_socket(None, &remote_name, &done_cb, deadline)
        } else {
            // Relative path
            if let Some(runtime_dir) = get_runtime_dir() {
                if self
                    .try_connect_local_socket(Some(&runtime_dir), &remote_name, &done_cb, deadline)
                    .is_ok()
                {
                    // Connect via runtime dir worked
                    return Ok(());
                }
            }

            // Fallback to connect via system dir
            self.try_connect_local_socket(Some(&get_system_dir()), &remote_name, &done_cb, deadline)
        }
    }

    fn try_connect_local_socket(
        &self,
        path: Option<&str>,
        name: &str,
        done_cb: &Option<Box<dyn Fn(std::io::Result<()>)>>,
        deadline: Option<std::time::Instant>,
    ) -> std::io::Result<()> {
        let mut socket_path = PathBuf::new();

        if let Some(path) = path {
            socket_path.push(path);
        }

        socket_path.push(name);

        debug!("Trying to connect to {:?}", socket_path);

        // Rust sockets are implicitly CLOEXEC
        let stream = match deadline {
            Some(deadline) => connect_before(&socket_path, deadline)?,
            None => UnixStream::connect(socket_path)?,
        };
        stream.set_nonblocking(true)?;

        let res = self.set_stream(stream);

        if let Some(cb) = done_cb {
            cb(res);
        }

        Ok(())
    }
}

impl InnerClient {
    fn new() -> Self {
        Self {
            core: RwLock::new(None),
            stream: RwLock::new(None),
            connection: Connection::new(None),
            connected: RwLock::new(false),
            need_flush: RwLock::new(false),
            last_in_seq: RwLock::new(0),
            dispatch_limits: RwLock::new(None),
            source: RwLock::new(None),
            hooks: RwLock::new(None),
        }
    }

    fn set_core(&self, core: WeakCore) {
        self.core.write().unwrap().replace(core);
    }
}

fn connect_before(
    path: &std::path::Path,
    deadline: std::time::Instant,
) -> std::io::Result<UnixStream> {
    if std::time::Instant::now() >= deadline {
        return Err(std::io::ErrorKind::TimedOut.into());
    }
    async_io::block_on(futures_lite::future::race(
        async {
            let stream = async_io::Async::<UnixStream>::connect(path).await?;
            if let Some(error) = stream.get_ref().take_error()? {
                return Err(error);
            }
            if std::time::Instant::now() >= deadline {
                return Err(std::io::ErrorKind::TimedOut.into());
            }
            stream.into_inner()
        },
        async {
            async_io::Timer::at(deadline).await;
            Err(std::io::ErrorKind::TimedOut.into())
        },
    ))
}

#[cfg(test)]
mod guarded_connect_tests {
    use super::*;
    #[test]
    fn saturated_listener_and_expired_connect_leave_no_later_connection() {
        use std::os::unix::net::UnixListener;
        use std::time::{Duration, Instant};
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("owned.sock");
        let listener = UnixListener::bind(&path).unwrap();
        listener.set_nonblocking(true).unwrap();
        // A small owned backlog makes pressure deterministic without any daemon.
        // SAFETY: this is the live listener descriptor; listen changes its backlog.
        assert_eq!(unsafe { libc::listen(listener.as_raw_fd(), 1) }, 0);
        let mut accepted = Vec::new();
        for _ in 0..16 {
            match connect_before(&path, Instant::now() + Duration::from_millis(20)) {
                Ok(stream) => accepted.push(stream),
                Err(_) => break,
            }
        }
        assert!(!accepted.is_empty() && accepted.len() < 16);
        let started = Instant::now();
        assert!(connect_before(&path, started + Duration::from_millis(20)).is_err());
        assert!(started.elapsed() < Duration::from_secs(1));
        assert!(connect_before(&path, Instant::now()).is_err());
        let mut drained = 0;
        while let Ok((_stream, _)) = listener.accept() {
            drained += 1;
        }
        assert_eq!(drained, accepted.len());
        // No abandoned future remains to connect later when backlog space opens.
        std::thread::sleep(Duration::from_millis(30));
        assert!(
            matches!(listener.accept(), Err(error) if error.kind() == std::io::ErrorKind::WouldBlock)
        );
        let fresh = connect_before(&path, Instant::now() + Duration::from_millis(100)).unwrap();
        assert!(listener.accept().is_ok());
        drop(fresh);
    }
}
