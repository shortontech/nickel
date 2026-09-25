use std::path::PathBuf;
#[cfg(any(test, target_os = "linux"))]
use std::sync::mpsc;

use nickel_file::{FileLaunch, FileWindowRequest};

pub(crate) trait FileWindowHost: Send + Sync {
    fn dispatch(&self, request: FileWindowRequest) -> Result<(), String>;
}

#[cfg(not(target_os = "windows"))]
pub(crate) struct ExternalFileWindowHost;

#[cfg(not(target_os = "windows"))]
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
            .with_file_name("nickel-file");
        let mut command = std::process::Command::new(executable);
        #[cfg(target_os = "linux")]
        command.env_remove("__EGL_VENDOR_LIBRARY_FILENAMES");
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

#[cfg(any(test, target_os = "linux"))]
pub(crate) struct ChannelFileWindowHost {
    sender: mpsc::Sender<FileWindowRequest>,
}

#[cfg(any(test, target_os = "linux"))]
impl FileWindowHost for ChannelFileWindowHost {
    fn dispatch(&self, request: FileWindowRequest) -> Result<(), String> {
        self.sender
            .send(request)
            .map_err(|_| "internal file-window owner stopped".into())
    }
}

#[cfg(any(test, target_os = "linux"))]
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
    #[cfg(target_os = "windows")]
    return std::sync::Arc::new(InProcessFileWindowHost);

    #[cfg(not(target_os = "windows"))]
    std::sync::Arc::new(ExternalFileWindowHost)
}

#[cfg(target_os = "windows")]
struct InProcessFileWindowHost;

#[cfg(target_os = "windows")]
impl FileWindowHost for InProcessFileWindowHost {
    fn dispatch(&self, request: FileWindowRequest) -> Result<(), String> {
        let launch = match request {
            FileWindowRequest::Open(launch) | FileWindowRequest::OpenOrFocus(launch) => launch,
            FileWindowRequest::Focus(_) | FileWindowRequest::Close(_) => {
                return Err("file window is not addressable by internal id".into());
            }
        };
        std::thread::Builder::new()
            .name("nickel-file-window".into())
            .spawn(move || {
                let _window_thread = crate::platform::register_internal_window_thread();
                let adapter = nickel_file::FileHostAdapter::default()
                    .with_focused_shortcut_handler(|key, edge| match key {
                        nickel_input::KeyCode::SuperLeft => {
                            crate::platform::observe_nickel_window_key(
                                Some(1),
                                edge == nickel_input::KeyEdge::Pressed,
                            );
                        }
                        nickel_input::KeyCode::SuperRight => {
                            crate::platform::observe_nickel_window_key(
                                Some(2),
                                edge == nickel_input::KeyEdge::Pressed,
                            );
                        }
                        _ => crate::platform::handle_focused_shortcut(key, edge),
                    });
                if let Err(error) =
                    nickel_ui::run_with_adapter_on_any_thread(launch.into_app(), adapter)
                {
                    tracing::error!(%error, "in-process file window failed");
                }
            })
            .map(|_| ())
            .map_err(|error| error.to_string())
    }
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
