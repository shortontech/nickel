use std::f32::consts::TAU;

use super::{Lod, Mesh, Vec3, Vertex, recompute_normals};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AnatomicalFrame {
    pub position: Vec3,
    pub forward: Vec3,
    pub up: Vec3,
    pub outward: Vec3,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SculptOperation {
    pub target: &'static str,
    pub kind: &'static str,
    pub amount: f32,
}

#[derive(Clone, Debug)]
pub struct SculptExperiment {
    pub strength: u8,
    pub shoulder: [AnatomicalFrame; 2],
    pub hip: [AnatomicalFrame; 2],
    pub history: Vec<SculptOperation>,
    pub mesh: Mesh,
}

pub fn generate_sculpt_experiment(strength: u8, lod: Lod) -> Result<SculptExperiment, String> {
    if !(1..=10).contains(&strength) {
        return Err("sculpt strength must be between 1 and 10".to_owned());
    }
    let force = (strength as f32 - 1.0) / 9.0;
    let shoulder_half_width = 0.50 + force * 0.18;
    let hip_half_width = 0.34 + force * 0.09;
    let arm_radius = 0.12 + force * 0.10;
    let leg_radius = 0.16 + force * 0.08;
    let outward = |side: f32| Vec3::new(side, 0.0, 0.0);
    let frame = |position, side| AnatomicalFrame {
        position,
        forward: Vec3::new(0.0, 0.0, -1.0),
        up: Vec3::new(0.0, 1.0, 0.0),
        outward: outward(side),
    };
    let shoulder = [
        frame(Vec3::new(-shoulder_half_width, 1.60, 0.0), -1.0),
        frame(Vec3::new(shoulder_half_width, 1.60, 0.0), 1.0),
    ];
    let hip = [
        frame(Vec3::new(-hip_half_width, 0.62, 0.0), -1.0),
        frame(Vec3::new(hip_half_width, 0.62, 0.0), 1.0),
    ];

    let history = vec![
        SculptOperation {
            target: "torso",
            kind: "loft",
            amount: 1.0,
        },
        SculptOperation {
            target: "shoulder_girdle",
            kind: "widen",
            amount: shoulder_half_width,
        },
        SculptOperation {
            target: "upper_limbs",
            kind: "sweep",
            amount: arm_radius,
        },
        SculptOperation {
            target: "support_limbs",
            kind: "sweep",
            amount: leg_radius,
        },
        SculptOperation {
            target: "deltoids",
            kind: "inflate",
            amount: force,
        },
        SculptOperation {
            target: "arm_torso_gap",
            kind: "preserve",
            amount: 0.16,
        },
        SculptOperation {
            target: "thigh_gap",
            kind: "preserve",
            amount: 0.12,
        },
        SculptOperation {
            target: "bulb_skull_gap",
            kind: "preserve",
            amount: 0.18,
        },
    ];

    let sides = match lod {
        Lod::Distant => 10,
        Lod::Gameplay => 16,
        Lod::Close => 24,
        Lod::Inspection => 32,
    };
    let clay = [151, 157, 166];
    let mut mesh = loft(
        &[
            (Vec3::new(0.0, 0.55, 0.0), 0.38 + force * 0.06, 0.30),
            (
                Vec3::new(0.0, 0.95, 0.0),
                0.46 + force * 0.08,
                0.35 + force * 0.04,
            ),
            (
                Vec3::new(0.0, 1.38, 0.0),
                0.52 + force * 0.15,
                0.34 + force * 0.06,
            ),
            (
                Vec3::new(0.0, 1.72, -0.02),
                0.43 + force * 0.10,
                0.31 + force * 0.04,
            ),
            (Vec3::new(0.0, 2.02, -0.08), 0.38 + force * 0.06, 0.35),
            (Vec3::new(0.0, 2.25, -0.10), 0.30, 0.31),
            (Vec3::new(0.0, 2.35, -0.10), 0.16, 0.18),
            (Vec3::new(0.0, 2.38, -0.10), 0.025, 0.03),
        ],
        sides,
        clay,
    );

    for side in [-1.0_f32, 1.0] {
        let shoulder_point = Vec3::new(side * shoulder_half_width, 1.60, 0.0);
        let elbow = shoulder_point + Vec3::new(side * 0.42, -0.12, 0.01);
        let wrist = shoulder_point + Vec3::new(side * 0.78, -0.22, -0.01);
        mesh.append(sweep(
            &[shoulder_point, elbow, wrist],
            &[arm_radius * 1.12, arm_radius, arm_radius * 0.72],
            sides,
            clay,
        ));

        let hip_point = Vec3::new(side * hip_half_width, 0.65, 0.0);
        let knee = Vec3::new(side * (0.30 + force * 0.05), 0.08, 0.04);
        let ankle = Vec3::new(side * (0.31 + force * 0.06), -0.52, 0.0);
        mesh.append(sweep(
            &[hip_point, knee, ankle],
            &[leg_radius * 1.12, leg_radius, leg_radius * 0.72],
            sides,
            clay,
        ));
        mesh.append(loft(
            &[
                (
                    Vec3::new(ankle.x, -0.58, -0.04),
                    leg_radius * 0.72,
                    leg_radius * 0.78,
                ),
                (
                    Vec3::new(ankle.x, -0.68, -0.17),
                    leg_radius * 1.05,
                    leg_radius * 1.55,
                ),
            ],
            sides,
            clay,
        ));
    }

    // A separate lofted dorsal feature intentionally preserves a readable skull gap.
    mesh.append(loft(
        &[
            (Vec3::new(0.0, 1.04, 0.33), 0.14, 0.10),
            (Vec3::new(0.0, 1.14, 0.41), 0.31, 0.22),
            (Vec3::new(0.0, 1.34, 0.48), 0.40, 0.28),
            (Vec3::new(0.0, 1.52, 0.43), 0.31, 0.23),
            (Vec3::new(0.0, 1.58, 0.38), 0.13, 0.10),
        ],
        sides,
        [105, 125, 104],
    ));
    recompute_normals(&mut mesh);
    Ok(SculptExperiment {
        strength,
        shoulder,
        hip,
        history,
        mesh,
    })
}

fn loft(sections: &[(Vec3, f32, f32)], sides: usize, color: [u8; 3]) -> Mesh {
    let mut mesh = Mesh::default();
    for &(center, radius_x, radius_z) in sections {
        for side in 0..sides {
            let angle = side as f32 / sides as f32 * TAU;
            mesh.vertices.push(Vertex {
                position: center + Vec3::new(angle.cos() * radius_x, 0.0, angle.sin() * radius_z),
                normal: Vec3::default(),
                color,
                surface_feature: [2.0; 3],
            });
        }
    }
    connect_rings(&mut mesh.indices, sections.len(), sides);
    cap(&mut mesh, sections[0].0, 0, sides, true, color);
    cap(
        &mut mesh,
        sections[sections.len() - 1].0,
        (sections.len() - 1) * sides,
        sides,
        false,
        color,
    );
    recompute_normals(&mut mesh);
    mesh
}

fn sweep(points: &[Vec3], radii: &[f32], sides: usize, color: [u8; 3]) -> Mesh {
    let mut mesh = Mesh::default();
    for (index, &center) in points.iter().enumerate() {
        let tangent = if index == 0 {
            points[1] - points[0]
        } else if index + 1 == points.len() {
            points[index] - points[index - 1]
        } else {
            points[index + 1] - points[index - 1]
        }
        .normalized();
        let reference = if tangent.y.abs() < 0.9 {
            Vec3::new(0.0, 1.0, 0.0)
        } else {
            Vec3::new(0.0, 0.0, 1.0)
        };
        let across = tangent.cross(reference).normalized();
        let around = tangent.cross(across).normalized();
        for side in 0..sides {
            let angle = side as f32 / sides as f32 * TAU;
            mesh.vertices.push(Vertex {
                position: center + (across * angle.cos() + around * angle.sin()) * radii[index],
                normal: Vec3::default(),
                color,
                surface_feature: [2.0; 3],
            });
        }
    }
    connect_rings(&mut mesh.indices, points.len(), sides);
    cap(&mut mesh, points[0], 0, sides, true, color);
    cap(
        &mut mesh,
        points[points.len() - 1],
        (points.len() - 1) * sides,
        sides,
        false,
        color,
    );
    recompute_normals(&mut mesh);
    mesh
}

fn connect_rings(indices: &mut Vec<u32>, rings: usize, sides: usize) {
    for ring in 0..rings - 1 {
        for side in 0..sides {
            let next = (side + 1) % sides;
            let a = (ring * sides + side) as u32;
            let b = (ring * sides + next) as u32;
            let c = ((ring + 1) * sides + side) as u32;
            let d = ((ring + 1) * sides + next) as u32;
            indices.extend_from_slice(&[a, c, b, b, c, d]);
        }
    }
}

fn cap(mesh: &mut Mesh, center: Vec3, ring: usize, sides: usize, reverse: bool, color: [u8; 3]) {
    let middle = mesh.vertices.len() as u32;
    mesh.vertices.push(Vertex {
        position: center,
        normal: Vec3::default(),
        color,
        surface_feature: [2.0; 3],
    });
    for side in 0..sides {
        let a = (ring + side) as u32;
        let b = (ring + (side + 1) % sides) as u32;
        if reverse {
            mesh.indices.extend_from_slice(&[middle, b, a]);
        } else {
            mesh.indices.extend_from_slice(&[middle, a, b]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strength_changes_structure_without_height() {
        let weak = generate_sculpt_experiment(1, Lod::Gameplay).unwrap();
        let strong = generate_sculpt_experiment(10, Lod::Gameplay).unwrap();
        assert!(strong.shoulder[1].position.x > weak.shoulder[1].position.x);
        let height = |mesh: &Mesh| {
            mesh.vertices
                .iter()
                .map(|v| v.position.y)
                .fold((f32::INFINITY, f32::NEG_INFINITY), |(min, max), y| {
                    (min.min(y), max.max(y))
                })
        };
        let weak_height = height(&weak.mesh);
        let strong_height = height(&strong.mesh);
        assert!(
            (weak_height.1 - weak_height.0 - (strong_height.1 - strong_height.0)).abs() < 0.001
        );
    }

    #[test]
    fn sculpt_history_is_deterministic() {
        let a = generate_sculpt_experiment(5, Lod::Gameplay).unwrap();
        let b = generate_sculpt_experiment(5, Lod::Gameplay).unwrap();
        assert_eq!(a.history, b.history);
        assert_eq!(a.mesh, b.mesh);
    }
}
