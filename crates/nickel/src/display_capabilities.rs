//! Public display observation identity shared by the capability and native owner.
use nickel_session_protocol::OutputSnapshot;
use std::hash::{Hash, Hasher};

pub(crate) fn revision(outputs: &[OutputSnapshot]) -> String {
    let mut ordered = outputs.iter().collect::<Vec<_>>();
    ordered.sort_by(|left, right| left.name.cmp(&right.name));
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    serde_json::to_string(&ordered)
        .expect("typed outputs serialize")
        .hash(&mut hash);
    format!("{:016x}", hash.finish())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn display_revision_tracks_orientation_and_ignores_inventory_order() {
        use nickel_session_protocol::{Geometry, OutputTransform};
        let output = OutputSnapshot {
            name: "left".into(),
            model: "fixture".into(),
            geometry: Geometry {
                x: 0,
                y: 0,
                width: 1920,
                height: 1080,
            },
            work_area: Geometry {
                x: 0,
                y: 0,
                width: 1920,
                height: 1080,
            },
            scale_120: 120,
            transform: OutputTransform::Normal,
            physical_width_mm: 500,
            physical_height_mm: 300,
            primary: true,
            enabled: true,
            modes: vec![],
            current_mode: None,
        };
        let mut second = output.clone();
        second.name = "right".into();
        second.primary = false;
        assert_eq!(
            revision(&[output.clone(), second.clone()]),
            revision(&[second, output.clone()])
        );
        let mut external = output.clone();
        external.name = "DP-1".into();
        external.primary = false;
        external.transform = OutputTransform::Rotate90;
        external.current_mode = Some(nickel_session_protocol::OutputMode {
            width: 1920,
            height: 1080,
            refresh_millihz: 60_000,
        });
        let mut internal = output.clone();
        internal.name = "eDP-1".into();
        let layout = super::projection_layout(
            &[internal.clone(), external.clone()],
            nickel_core::display_projection::ProjectionMode::ExternalOnly,
        )
        .unwrap();
        assert_eq!(layout.primary, "DP-1");
        assert_eq!(layout.placements[1].mode, external.current_mode);
        assert_eq!(
            layout.placements[1].transform,
            Some(OutputTransform::Rotate90)
        );
        assert!(!layout.placements[0].enabled);
        assert!(
            super::projection_layout(
                &[internal],
                nickel_core::display_projection::ProjectionMode::Extend
            )
            .is_none()
        );
        let mut rotated = output.clone();
        rotated.transform = OutputTransform::Rotate90;
        assert_ne!(revision(&[output]), revision(&[rotated]));
    }
}

pub(crate) fn projection_mode(id: &str) -> Option<nickel_core::display_projection::ProjectionMode> {
    use nickel_core::display_projection::ProjectionMode;
    match id {
        "internal" => Some(ProjectionMode::InternalOnly),
        "duplicate" => Some(ProjectionMode::Duplicate),
        "extend" => Some(ProjectionMode::Extend),
        "external" => Some(ProjectionMode::ExternalOnly),
        _ => None,
    }
}
pub(crate) fn projection_modes(outputs: &[OutputSnapshot]) -> serde_json::Value {
    use nickel_core::display_projection::{ProjectionChooser, ProjectionMode};
    let topology = projection_outputs(outputs);
    serde_json::Value::Array(
        ProjectionChooser::supported(&topology)
            .into_iter()
            .map(|mode| {
                let (id, label) = match mode {
                    ProjectionMode::InternalOnly => ("internal", "Internal"),
                    ProjectionMode::Duplicate => ("duplicate", "Duplicate"),
                    ProjectionMode::Extend => ("extend", "Extend"),
                    ProjectionMode::ExternalOnly => ("external", "External"),
                };
                serde_json::json!({"id":id,"label":label})
            })
            .collect(),
    )
}
fn projection_outputs(
    outputs: &[OutputSnapshot],
) -> Vec<nickel_core::display_projection::ProjectionOutput> {
    outputs
        .iter()
        .map(|output| nickel_core::display_projection::ProjectionOutput {
            name: output.name.clone(),
            internal: output.name.starts_with("eDP") || output.name.starts_with("LVDS"),
            width: output.geometry.width,
            height: output.geometry.height,
            scale: nickel_core::dpi::Scale120::new(output.scale_120).unwrap_or_default(),
        })
        .collect()
}
pub(crate) fn projection_layout(
    outputs: &[OutputSnapshot],
    mode: nickel_core::display_projection::ProjectionMode,
) -> Option<nickel_session_protocol::OutputLayout> {
    let plan = nickel_core::display_projection::ProjectionChooser::plan(
        mode,
        &projection_outputs(outputs),
    )?;
    let placements = plan
        .placements
        .iter()
        .map(|entry| {
            let output = outputs
                .iter()
                .find(|output| output.name == entry.name)
                .unwrap();
            nickel_session_protocol::OutputPlacement {
                name: entry.name.clone(),
                x: entry.x,
                y: entry.y,
                enabled: entry.enabled,
                scale_120: entry.scale.units(),
                mode: output.current_mode.clone(),
                transform: Some(output.transform),
            }
        })
        .collect::<Vec<_>>();
    let primary = outputs
        .iter()
        .find(|output| {
            output.primary
                && placements
                    .iter()
                    .any(|entry| entry.name == output.name && entry.enabled)
        })
        .or_else(|| {
            outputs.iter().find(|output| {
                placements
                    .iter()
                    .any(|entry| entry.name == output.name && entry.enabled)
            })
        })?
        .name
        .clone();
    Some(nickel_session_protocol::OutputLayout {
        primary,
        placements,
    })
}
