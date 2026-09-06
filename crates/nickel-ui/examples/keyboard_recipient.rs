//! Native keyboard/controller ownership probe. Run inside an isolated Nickel test session.
use nickel_ui::{
    Application, Button, Column, ComponentBuilderExt, Container, ControllerFence, HostAdapter,
    HostServices, Insets, Text, TextField, ViewContext,
};

#[derive(Clone, Debug)]
enum Message {
    Activate,
    Text(String),
}
#[derive(Default)]
struct Recipient {
    actions: usize,
    text: String,
}

#[derive(Default)]
struct SessionAdapter;

impl HostAdapter<Recipient> for SessionAdapter {
    fn controller_fence(&mut self, _services: HostServices<'_>) -> ControllerFence {
        #[cfg(target_os = "linux")]
        {
            use nickel_session_protocol::{Query, Request, ServerMessage};
            match nickel_session_protocol::client::request_from_environment(
                Request::Query(Query::OnScreenKeyboard),
                std::time::Duration::from_millis(25),
            ) {
                Ok(None) => ControllerFence::default(),
                Ok(Some(ServerMessage::OnScreenKeyboard(snapshot))) => ControllerFence {
                    blocked: snapshot.visible,
                    barrier_unix_ms: snapshot.controller_barrier_unix_ms,
                },
                _ => ControllerFence {
                    blocked: true,
                    ..ControllerFence::default()
                },
            }
        }
        #[cfg(not(target_os = "linux"))]
        ControllerFence::default()
    }

    fn request_text_entry(&mut self, _services: HostServices<'_>) {
        #[cfg(target_os = "linux")]
        {
            use nickel_session_protocol::{Command, Request};
            let _ = nickel_session_protocol::client::request_from_environment(
                Request::Command(Command::RequestOnScreenKeyboard),
                std::time::Duration::from_millis(25),
            );
        }
    }
}
impl Application for Recipient {
    type Message = Message;
    fn title(&self) -> &str {
        "Keyboard recipient acceptance"
    }
    fn initial_size(&self) -> (u32, u32) {
        (720, 220)
    }
    fn update(&mut self, message: Message) {
        match message {
            Message::Activate => {
                self.actions += 1;
                println!("recipient-actions={}", self.actions);
            }
            Message::Text(text) => {
                self.text = text;
                println!("recipient-text={}", self.text);
            }
        }
    }
    fn view(&self, _: ViewContext) -> impl nickel_ui::View<Message> {
        Container::new().padding(Insets::all(16.0)).child(
            Column::new()
                .gap(16.0)
                .child(Text::new("Keyboard recipient acceptance"))
                .child(
                    TextField::on_change(&self.text, Message::Text)
                        .id("recipient-text")
                        .accessibility_label("Recipient text")
                        .width(680.0)
                        .height(44.0),
                )
                .child(
                    Button::new(
                        Message::Activate,
                        format!("Recipient actions: {}", self.actions),
                    )
                    .id("recipient-action")
                    .width(680.0)
                    .height(48.0),
                ),
        )
    }
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    #[cfg(target_os = "linux")]
    if std::env::args().nth(1).as_deref() == Some("--bootstrap-two-outputs") {
        use nickel_session_protocol::{
            Command, OutputTransform, Request, ServerMessage, TestOutput,
        };
        use std::os::unix::process::CommandExt;
        let shell = std::env::args_os()
            .nth(2)
            .ok_or("expected shell executable")?;
        let result = nickel_session_protocol::client::request_from_environment(
            Request::Command(Command::TestOutput {
                output: TestOutput::Connect {
                    name: "osk-secondary".into(),
                    logical_width: 1280,
                    logical_height: 720,
                    scale_120: 120,
                    transform: OutputTransform::Normal,
                },
            }),
            std::time::Duration::from_secs(2),
        )?;
        if !matches!(result, Some(ServerMessage::Ack)) {
            return Err("two-output bootstrap requires an isolated test-control session".into());
        }
        // Preserve the PID expected by the compositor's shell authentication.
        return Err(std::process::Command::new(shell).exec().into());
    }
    nickel_ui::run_with_adapter(Recipient::default(), SessionAdapter)
}
