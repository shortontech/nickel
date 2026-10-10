//! Host-provided presentation bounds. These descriptors grant no package or native authority.
//! Hosts validate ownership and permissions before supplying them and before applying requests.
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct SurfaceDefinition {
    /// Ordinary surfaces open at activation by default. Transients stay explicit.
    #[serde(default = "default_true", skip_serializing_if = "is_true")]
    pub initially_open: bool,
    pub id: String,
    pub kind: SurfaceKind,
    pub width: u32,
    pub height: u32,
    #[serde(default)]
    pub bottom_offset: u32,
    #[serde(default)]
    pub reserve_work_area: bool,
    #[serde(default)]
    pub output: OutputScope,
    #[serde(default, skip_serializing_if = "SurfaceAnchor::is_center")]
    pub anchor: SurfaceAnchor,
    #[serde(default, skip_serializing_if = "is_zero_i32")]
    pub offset_x: i32,
    #[serde(default, skip_serializing_if = "is_zero_i32")]
    pub offset_y: i32,
    #[serde(default, skip_serializing_if = "is_false")]
    pub passive: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner: Option<String>,
}

fn default_true() -> bool {
    true
}
fn is_true(value: &bool) -> bool {
    *value
}

fn is_zero_i32(value: &i32) -> bool {
    *value == 0
}

fn is_false(value: &bool) -> bool {
    !*value
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum SurfaceAnchor {
    #[default]
    Center,
    TopLeft,
    TopCenter,
    TopRight,
    BottomLeft,
    BottomCenter,
    BottomRight,
}

impl SurfaceAnchor {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Center => "center",
            Self::TopLeft => "top-left",
            Self::TopCenter => "top-center",
            Self::TopRight => "top-right",
            Self::BottomLeft => "bottom-left",
            Self::BottomCenter => "bottom-center",
            Self::BottomRight => "bottom-right",
        }
    }

    pub fn is_center(&self) -> bool {
        *self == Self::Center
    }

    pub fn position(
        self,
        output: (i32, i32, u32, u32),
        size: (u32, u32),
        offset: (i32, i32),
    ) -> (i32, i32) {
        let (x, y, output_width, output_height) = output;
        let (width, height) = size;
        let remaining_x = output_width.saturating_sub(width).min(i32::MAX as u32) as i32;
        let remaining_y = output_height.saturating_sub(height).min(i32::MAX as u32) as i32;
        let anchor_x = match self {
            Self::TopLeft | Self::BottomLeft => 0,
            Self::TopRight | Self::BottomRight => remaining_x,
            Self::Center | Self::TopCenter | Self::BottomCenter => remaining_x / 2,
        };
        let anchor_y = match self {
            Self::TopLeft | Self::TopCenter | Self::TopRight => 0,
            Self::BottomLeft | Self::BottomCenter | Self::BottomRight => remaining_y,
            Self::Center => remaining_y / 2,
        };
        (
            x.saturating_add(anchor_x.saturating_add(offset.0))
                .clamp(x, x.saturating_add(remaining_x)),
            y.saturating_add(anchor_y.saturating_add(offset.1))
                .clamp(y, y.saturating_add(remaining_y)),
        )
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum SurfaceKind {
    Panel,
    Dock,
    Desktop,
    Window,
    Dialog,
    Overlay,
}

impl SurfaceKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Panel => "panel",
            Self::Dock => "dock",
            Self::Desktop => "desktop",
            Self::Window => "window",
            Self::Dialog => "dialog",
            Self::Overlay => "overlay",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum OutputScope {
    #[default]
    Primary,
    Active,
    All,
}

/// Supplies the surface descriptors a host permits the current materialization to request.
pub trait SurfaceBoundsProvider {
    fn surface_bounds(&self) -> &[SurfaceDefinition];
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct SurfaceBounds {
    #[serde(default)]
    pub surfaces: Vec<SurfaceDefinition>,
}

impl SurfaceBounds {
    pub fn snapshot(provider: &impl SurfaceBoundsProvider) -> Self {
        Self {
            surfaces: provider.surface_bounds().to_vec(),
        }
    }

    /// Deserializes descriptors only; callers remain responsible for host admission.
    pub fn from_json(source: &str) -> Result<Self, String> {
        serde_json::from_str(source).map_err(|error| error.to_string())
    }
}

impl SurfaceBoundsProvider for SurfaceBounds {
    fn surface_bounds(&self) -> &[SurfaceDefinition] {
        &self.surfaces
    }
}
