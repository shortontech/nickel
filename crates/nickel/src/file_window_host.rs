use std::{path::PathBuf, sync::mpsc};

use nickel_file::{FileLaunch, FileWindowRequest};

pub(crate) trait FileWindowHost: Send + Sync {
    fn dispatch(&self, request: FileWindowRequest) -> Result<(), String>;
}

pub(crate) struct ExternalFileWindowHost;

impl FileWindowHost for ExternalFileWindowHost {
    fn dispatch(&self, request: FileWindowRequest) -> Result<(), String> {
        let launch = match request {
            FileWindowRequest::Open(launch) | FileWindowRequest::OpenOrFocus(launch) => launch,
            FileWindowRequest::Focus(_) | FileWindowRequest::Close(_) => {
                return Err("external file windows cannot be addressed by internal id".into());
            }
        };
        let executable = std::env::current_exe()
            .map_err(|error| error.to_string())?
            .with_file_name(if cfg!(target_os = "windows") {
                "nickel-file.exe"
            } else {
                "nickel-file"
            });
        let mut command = std::process::Command::new(executable);
        match launch {
            FileLaunch::Browse(path) => command.arg(path),
            FileLaunch::Properties(path) => command.arg("--properties").arg(path),
            FileLaunch::Rename(path) => command.arg("--rename").arg(path),
        };
        command
            .spawn()
            .map(|_| ())
            .map_err(|error| error.to_string())
    }
}

pub(crate) struct ChannelFileWindowHost {
    sender: mpsc::Sender<FileWindowRequest>,
}

impl FileWindowHost for ChannelFileWindowHost {
    fn dispatch(&self, request: FileWindowRequest) -> Result<(), String> {
        self.sender
            .send(request)
            .map_err(|_| "internal file-window owner stopped".into())
    }
}

pub(crate) fn internal_file_window_channel() -> (
    std::sync::Arc<dyn FileWindowHost>,
    mpsc::Receiver<FileWindowRequest>,
) {
    let (sender, receiver) = mpsc::channel();
    (
        std::sync::Arc::new(ChannelFileWindowHost { sender }),
        receiver,
    )
}

pub(crate) fn browse_request(path: PathBuf) -> FileWindowRequest {
    FileWindowRequest::OpenOrFocus(FileLaunch::Browse(path))
}

pub(crate) fn default_file_window_host() -> std::sync::Arc<dyn FileWindowHost> {
    std::sync::Arc::new(ExternalFileWindowHost)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn internal_host_preserves_typed_file_launch_request() {
        let (host, receiver) = internal_file_window_channel();
        let request = browse_request(PathBuf::from("/home/example/Documents"));

        host.dispatch(request.clone()).unwrap();

        assert_eq!(receiver.recv().unwrap(), request);
    }
}
