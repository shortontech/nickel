use schemars::JsonSchema;
use serde::Deserialize;

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
