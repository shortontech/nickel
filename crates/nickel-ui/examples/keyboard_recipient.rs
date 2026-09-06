//! Native keyboard/controller ownership probe. Run inside an isolated Nickel test session.
use nickel_ui::{
    Application, Button, Column, ComponentBuilderExt, Container, Insets, Text, TextField,
    ViewContext,
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
    nickel_ui::run(Recipient::default())
}
