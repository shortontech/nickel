use nickel_ui::backend::PaintCommand;
use nickel_ui::{
    ActionKind, ComponentBuilderExt, CustomPaint, Rect, Row, SemanticAction, SemanticRole,
    SemanticSelector, Spacer, UiFrame,
};

#[derive(Clone, Debug, PartialEq)]
enum Message {
    Activate,
}

#[test]
fn precomputed_custom_commands_are_local_clipped_and_cannot_emit_overlays() {
    let frame = UiFrame::<()>::layout(
        Row::new().child(Spacer::new().width(10.0)).child(
            CustomPaint::commands(vec![
                PaintCommand::Fill {
                    rect: Rect::new(0.0, 0.0, 20.0, 20.0),
                    color: 0x123456,
                },
                PaintCommand::Fill {
                    rect: Rect::new(21.0, 0.0, 1.0, 1.0),
                    color: 0x654321,
                },
                PaintCommand::OverlayFill {
                    rect: Rect::new(0.0, 0.0, 20.0, 20.0),
                    color: 0xffffff,
                },
            ])
            .width(20.0)
            .height(20.0),
        ),
        Rect::new(0.0, 0.0, 30.0, 20.0),
    );
    assert!(frame.commands().iter().any(|command| matches!(
        command,
        PaintCommand::Fill { rect, color }
            if *color == 0x123456 && *rect == Rect::new(10.0, 0.0, 20.0, 20.0)
    )));
    assert!(!frame.commands().iter().any(|command| matches!(
        command,
        PaintCommand::Fill { color, .. } if *color == 0x654321
    )));
    assert!(
        !frame
            .commands()
            .iter()
            .any(|command| matches!(command, PaintCommand::OverlayFill { .. }))
    );
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
