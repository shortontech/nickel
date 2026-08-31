use nickel_ui::prelude::*;

#[derive(Default)]
struct Counter {
    value: i32,
}

#[derive(Clone)]
enum Message {
    Increment,
    Decrement,
}

impl Application for Counter {
    type Message = Message;

    fn update(&mut self, message: Message) {
        match message {
            Message::Increment => self.value += 1,
            Message::Decrement => self.value -= 1,
        }
    }

    fn view(&self, _context: nickel_ui::ViewContext) -> impl View<Message> {
        ui! {
            <Column id={id!(counter)} gap={12.0} padding={Insets::all(24.0)}>
                <Text scale={2.0}>{format!("Count: {}", self.value)}</Text>
                <Row gap={8.0}>
                    <Button on_press={Message::Decrement}>{"−"}</Button>
                    <Button on_press={Message::Increment}>{"+"}</Button>
                </Row>
            </Column>
        }
    }

    fn title(&self) -> &str {
        "Nickel UI Counter"
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    run(Counter::default())
}
