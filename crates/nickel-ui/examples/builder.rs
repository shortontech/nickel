use nickel_ui::{Button, Column, Constraints, Insets, Row, Size, Spacer, Text};

#[derive(Clone)]
enum Message {
    Save,
}

fn main() {
    let view = Column::new()
        .gap(12.0)
        .padding(Insets::all(20.0))
        .child(Text::new("Builder API"))
        .child(
            Row::new()
                .child(Spacer::flex())
                .child(Button::new(Message::Save, "Save")),
        );

    let measured = nickel_ui::Component::into_element(view)
        .measure(Constraints::loose(Size::new(640.0, 480.0)));
    assert!(measured.width >= 0.0 && measured.height >= 0.0);
}
