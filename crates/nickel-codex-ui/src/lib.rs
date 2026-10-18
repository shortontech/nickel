mod attachments;
mod controller;
mod model;
mod projection_memory;
mod view;

pub use attachments::{
    AttachmentError, AttachmentId, AttachmentLimits, ClipboardOffer, ClipboardPaste,
    PendingAttachment,
};
pub use controller::{
    BackendMode, ChatController, ControllerCommand, ControllerEvent, create_managed_workspace,
};
pub use model::{ChatItem, ChatItemKind, ChatState, ConnectionStatus, PendingInteraction};
pub use view::{
    ChatApplication, ChatMessage, CodexApprovalChoice, CodexApprovalNotification, ShellRequest,
};

/// Supported logical client minimum shared by the Winit and internal chat hosts.
pub const CHAT_MINIMUM_LOGICAL_SIZE: (u32, u32) = (640, 480);

pub fn shell_application(
    cwd: std::path::PathBuf,
    project_menu: bool,
    thread: Option<nickel_codex::ThreadId>,
    project_id: Option<String>,
) -> Result<ChatApplication, String> {
    shell_application_with_backend(cwd, project_menu, thread, project_id, None)
}

pub fn shell_application_with_backend(
    cwd: std::path::PathBuf,
    project_menu: bool,
    thread: Option<nickel_codex::ThreadId>,
    project_id: Option<String>,
    backend: Option<nickel_codex::BackendChoice>,
) -> Result<ChatApplication, String> {
    let settings_path =
        nickel_codex::CodexSettings::default_path().map_err(|error| error.to_string())?;
    let settings =
        nickel_codex::CodexSettings::load(&settings_path).map_err(|error| error.to_string())?;
    let mode = backend.map_or_else(
        || {
            settings.selected_host().map_or_else(
                || BackendMode::Live {
                    choice: nickel_codex::BackendChoice::Automatic,
                    cwd: cwd.clone(),
                },
                |host| BackendMode::Remote { host: host.clone() },
            )
        },
        |choice| BackendMode::Live {
            choice,
            cwd: cwd.clone(),
        },
    );
    let mut application = ChatApplication::with_settings(mode, settings, Some(settings_path));
    application = if project_menu {
        application.as_shell_project_menu()
    } else {
        application.as_shell_chat(&cwd)
    };
    if let Some(project_id) = project_id {
        application.use_project(cwd, project_id);
    }
    if let Some(thread) = thread {
        application.resume_thread(thread)?;
    }
    Ok(application)
}

#[cfg(test)]
mod tests;
