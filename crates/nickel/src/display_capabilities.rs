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
        let mut rotated = output.clone();
        rotated.transform = OutputTransform::Rotate90;
        assert_ne!(revision(&[output]), revision(&[rotated]));
    }
}
