use crate::{OverlayAnchor, OverlayMenu, OverlayMenuItem, TextEditor, UiId};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TextEditCommand {
    Undo,
    Redo,
    Cut,
    Copy,
    Paste,
    Delete,
    SelectAll,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TextContextPolicy {
    pub editable: bool,
    pub secure: bool,
    pub clipboard_has_text: bool,
}

impl Default for TextContextPolicy {
    fn default() -> Self {
        Self {
            editable: true,
            secure: false,
            clipboard_has_text: false,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TextContextAction {
    pub command: TextEditCommand,
    pub label: &'static str,
    pub enabled: bool,
    pub shortcut: &'static str,
}

/// Payload-free shared model used by every context-menu invocation route.
/// Secure text and clipboard payloads never enter labels or diagnostics.
pub fn text_context_actions(
    editor: &TextEditor,
    policy: TextContextPolicy,
) -> Vec<TextContextAction> {
    let (undo, redo, cut, copy, paste, select_all) = (
        "Ctrl+Z",
        "Ctrl+Shift+Z",
        "Ctrl+X",
        "Ctrl+C",
        "Ctrl+V",
        "Ctrl+A",
    );
    let selection = editor.selection().is_some();
    let action = |command, label, enabled, shortcut| TextContextAction {
        command,
        label,
        enabled,
        shortcut,
    };
    vec![
        action(
            TextEditCommand::Undo,
            "Undo",
            policy.editable && editor.can_undo(),
            undo,
        ),
        action(
            TextEditCommand::Redo,
            "Redo",
            policy.editable && editor.can_redo(),
            redo,
        ),
        action(
            TextEditCommand::Cut,
            "Cut",
            policy.editable && !policy.secure && selection,
            cut,
        ),
        action(
            TextEditCommand::Copy,
            "Copy",
            !policy.secure && selection,
            copy,
        ),
        action(
            TextEditCommand::Paste,
            "Paste",
            policy.editable && policy.clipboard_has_text,
            paste,
        ),
        action(
            TextEditCommand::Delete,
            "Delete",
            policy.editable && selection,
            "Delete",
        ),
        action(
            TextEditCommand::SelectAll,
            "Select All",
            !editor.text().is_empty(),
            select_all,
        ),
    ]
}

/// Builds the canonical action menu. Applications provide only the typed
/// command mapper; labels, ordering, shortcuts, and enablement stay shared.
pub fn text_context_menu<Message: Clone>(
    id: impl Into<UiId>,
    anchor: OverlayAnchor,
    editor: &TextEditor,
    policy: TextContextPolicy,
    map: fn(TextEditCommand) -> Message,
) -> OverlayMenu<Message> {
    text_context_actions(editor, policy).into_iter().fold(
        OverlayMenu::new(id, anchor),
        |menu, action| {
            let item = if action.enabled {
                OverlayMenuItem::action(
                    format!("{:?}", action.command).to_ascii_lowercase(),
                    action.label,
                    map(action.command),
                )
            } else {
                OverlayMenuItem::disabled(
                    format!("{:?}", action.command).to_ascii_lowercase(),
                    action.label,
                )
            };
            menu.item(item.shortcut(action.shortcut))
        },
    )
}

pub(crate) fn internal_text_context_menu<Message: Clone>(
    id: impl Into<UiId>,
    anchor: OverlayAnchor,
    editor: &TextEditor,
    policy: TextContextPolicy,
) -> OverlayMenu<Message> {
    text_context_actions(editor, policy).into_iter().fold(
        OverlayMenu::new(id, anchor),
        |menu, action| {
            menu.item(
                OverlayMenuItem::text_command(
                    format!("{:?}", action.command).to_ascii_lowercase(),
                    action.label,
                    action.command,
                    action.enabled,
                )
                .shortcut(action.shortcut)
                .separator_before(matches!(
                    action.command,
                    TextEditCommand::Cut | TextEditCommand::SelectAll
                )),
            )
        },
    )
}

pub(crate) fn internal_read_only_text_context_menu<Message: Clone>(
    id: impl Into<UiId>,
    anchor: OverlayAnchor,
    copy_enabled: bool,
    select_all_enabled: bool,
) -> OverlayMenu<Message> {
    let (copy, select_all) = ("Ctrl+C", "Ctrl+A");
    OverlayMenu::new(id, anchor)
        .item(
            OverlayMenuItem::text_command("copy", "Copy", TextEditCommand::Copy, copy_enabled)
                .shortcut(copy),
        )
        .item(
            OverlayMenuItem::text_command(
                "select-all",
                "Select All",
                TextEditCommand::SelectAll,
                select_all_enabled,
            )
            .shortcut(select_all)
            .separator_before(true),
        )
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct TextCommandEffect {
    pub changed: bool,
    pub clipboard_text: Option<String>,
}

/// Re-checks capability at activation time, then dispatches the production
/// editor command. This makes stale menus and clipboard changes harmless.
pub fn execute_text_command(
    editor: &mut TextEditor,
    mut policy: TextContextPolicy,
    command: TextEditCommand,
    clipboard_text: Option<&str>,
) -> TextCommandEffect {
    if command == TextEditCommand::Paste {
        policy.clipboard_has_text &= clipboard_text.is_some();
    }
    let enabled = text_context_actions(editor, policy)
        .into_iter()
        .find(|action| action.command == command)
        .is_some_and(|action| action.enabled);
    if !enabled {
        return TextCommandEffect::default();
    }
    let before = editor.text().to_owned();
    let mut copied = None;
    match command {
        TextEditCommand::Undo => editor.undo(),
        TextEditCommand::Redo => editor.redo(),
        TextEditCommand::Cut => {
            return TextCommandEffect {
                changed: true,
                clipboard_text: editor.cut_selection(),
            };
        }
        TextEditCommand::Copy => copied = editor.selected_text().map(ToOwned::to_owned),
        TextEditCommand::Paste => {
            let normalized = clipboard_text
                .unwrap_or_default()
                .replace("\r\n", "\n")
                .replace('\r', "\n");
            editor.insert(&normalized);
        }
        TextEditCommand::Delete => editor.delete_selected(),
        TextEditCommand::SelectAll => editor.select_all(),
    }
    TextCommandEffect {
        changed: editor.text() != before,
        clipboard_text: copied,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn enabled(actions: &[TextContextAction], command: TextEditCommand) -> bool {
        actions
            .iter()
            .find(|action| action.command == command)
            .unwrap()
            .enabled
    }

    #[test]
    fn availability_tracks_selection_history_clipboard_and_editability() {
        let mut editor = TextEditor::new("hello");
        let policy = TextContextPolicy {
            clipboard_has_text: true,
            ..Default::default()
        };
        let actions = text_context_actions(&editor, policy);
        assert!(!enabled(&actions, TextEditCommand::Copy));
        assert!(enabled(&actions, TextEditCommand::Paste));
        editor.select_all();
        editor.insert("world");
        assert!(enabled(
            &text_context_actions(&editor, policy),
            TextEditCommand::Undo
        ));
        let read_only = TextContextPolicy {
            editable: false,
            ..policy
        };
        assert!(!enabled(
            &text_context_actions(&editor, read_only),
            TextEditCommand::Paste
        ));
    }

    #[test]
    fn secure_fields_never_expose_or_copy_content() {
        let mut editor = TextEditor::new("correct horse battery staple");
        editor.select_all();
        let policy = TextContextPolicy {
            secure: true,
            clipboard_has_text: true,
            ..Default::default()
        };
        let actions = text_context_actions(&editor, policy);
        assert!(!enabled(&actions, TextEditCommand::Copy));
        assert!(!enabled(&actions, TextEditCommand::Cut));
        assert_eq!(
            execute_text_command(&mut editor, policy, TextEditCommand::Copy, None),
            TextCommandEffect::default()
        );
    }

    #[test]
    fn multiline_graphemes_round_trip_through_history() {
        let original = "one\ne\u{301}🦀";
        let mut editor = TextEditor::new(original);
        editor.select_all();
        let policy = TextContextPolicy {
            clipboard_has_text: true,
            ..Default::default()
        };
        let cut = execute_text_command(&mut editor, policy, TextEditCommand::Cut, None);
        assert_eq!(cut.clipboard_text.as_deref(), Some(original));
        execute_text_command(&mut editor, policy, TextEditCommand::Undo, None);
        assert_eq!(editor.text(), original);
        execute_text_command(&mut editor, policy, TextEditCommand::Redo, None);
        assert_eq!(editor.text(), "");
    }

    #[test]
    fn stale_disabled_activation_has_no_effect() {
        let mut editor = TextEditor::new("hello");
        assert_eq!(
            execute_text_command(
                &mut editor,
                TextContextPolicy::default(),
                TextEditCommand::Paste,
                Some("payload")
            ),
            TextCommandEffect::default()
        );
        assert_eq!(editor.text(), "hello");
    }

    #[derive(Clone, Debug, Eq, PartialEq)]
    struct Message(TextEditCommand);

    #[test]
    fn shared_menu_keeps_conventional_order_and_disabled_rows_visible() {
        let menu = text_context_menu(
            "editor-menu",
            OverlayAnchor::InvocationTarget("editor".into()),
            &TextEditor::new("text"),
            TextContextPolicy::default(),
            Message,
        );
        assert_eq!(
            menu.items
                .iter()
                .map(|item| item.label.as_str())
                .collect::<Vec<_>>(),
            [
                "Undo",
                "Redo",
                "Cut",
                "Copy",
                "Paste",
                "Delete",
                "Select All"
            ]
        );
        assert!(menu.items[..6].iter().all(|item| item.action.is_none()));
        assert_eq!(
            menu.items[6].action,
            Some(Message(TextEditCommand::SelectAll))
        );
    }
}
