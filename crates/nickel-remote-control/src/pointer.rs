use schemars::JsonSchema;
use serde::Deserialize;

/// A pointer coordinate space whose membership is resolved by the desktop owner
/// immediately before every event. Window and surface coordinates are local;
/// output and desktop coordinates are compositor-global.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum PointerTarget {
    Window { window_id: String, generation: u64 },
    Surface { surface_id: String, generation: u64 },
    Output { output_id: String, generation: u64 },
    Desktop,
}

impl PointerTarget {
    pub fn validate(&self) -> Result<(), String> {
        let identity = match self {
            Self::Window { window_id: id, .. }
            | Self::Surface { surface_id: id, .. }
            | Self::Output { output_id: id, .. } => Some(id),
            Self::Desktop => None,
        };
        if identity
            .is_some_and(|id| id.is_empty() || id.len() > 128 || id.chars().any(char::is_control))
        {
            return Err("pointer target identity is invalid or exceeds limit".into());
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PointerButton {
    Left,
    Right,
    Middle,
}

#[derive(Clone, Copy, Debug, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PointerAction {
    Move,
    Click {
        button: PointerButton,
    },
    DoubleClick {
        button: PointerButton,
    },
    DragStart {
        button: PointerButton,
    },
    DragMove,
    DragEnd,
    DragCancel,
    Scroll {
        horizontal_v120: i32,
        vertical_v120: i32,
    },
}

impl PointerAction {
    pub fn validate(self) -> Result<(), String> {
        if let Self::Scroll {
            horizontal_v120,
            vertical_v120,
        } = self
            && (horizontal_v120.unsigned_abs() > 12000 || vertical_v120.unsigned_abs() > 12000)
        {
            return Err("scroll amount exceeds operation limit".into());
        }
        Ok(())
    }
}
