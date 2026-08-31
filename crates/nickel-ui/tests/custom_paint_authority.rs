use nickel_ui::backend::PaintCommand;
use nickel_ui::{
    ActionKind, CustomPaint, Rect, SemanticAction, SemanticRole, SemanticSelector, UiFrame,
};

#[derive(Clone, Debug, PartialEq)]
enum Message {
    Activate,
}

fn paint(bounds: Rect) -> Vec<PaintCommand> {
    vec![PaintCommand::Fill {
        rect: bounds,
        color: 0x8b5cf6,
    }]
}

#[test]
fn bounded_custom_paint_keeps_semantics_accessibility_action_and_paint_on_one_node() {
    let frame = UiFrame::layout(
        CustomPaint::new(paint)
            .id("graph")
            .width(160.0)
            .height(80.0)
            .semantic_role(SemanticRole::Button)
            .accessibility_label("Open graph")
            .accessibility_description("Graphical preview")
            .message(Message::Activate),
        Rect::new(0.0, 0.0, 160.0, 80.0),
    );
    let semantic = frame
        .query_unique(&SemanticSelector::RoleAndName {
            role: SemanticRole::Button,
            name: "Open graph".into(),
        })
        .expect("custom painter semantic node");
    let accessibility = frame
        .accessibility_nodes()
        .iter()
        .find(|node| node.id == semantic.id)
        .expect("custom painter accessibility node");
    assert_eq!(accessibility.rect, semantic.bounds);
    assert_eq!(accessibility.label.as_deref(), semantic.name.as_deref());
    assert_eq!(
        accessibility.description.as_deref(),
        Some("Graphical preview")
    );
    assert!(frame.commands().iter().any(
        |command| matches!(command, PaintCommand::Fill { rect, .. } if *rect == semantic.bounds)
    ));
    assert_eq!(
        frame
            .perform_semantic_action(&semantic.id, SemanticAction::Invoke(ActionKind::Activate),)
            .expect("custom painter action")
            .messages,
        vec![Message::Activate]
    );
}
