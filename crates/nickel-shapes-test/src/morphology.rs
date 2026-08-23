use serde::{Deserialize, Serialize};

use crate::{BackKind, CapabilityLevel, ComponentUse, HeadKind, TorsoKind, Vec3};

pub const GENERATOR_VERSION: u32 = 4;

#[derive(Clone, Copy, Debug, Deserialize, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DietKind {
    Herbivore,
    Carnivore,
}

#[derive(Clone, Copy, Debug, Deserialize, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LocomotionKind {
    Quadruped,
    Biped,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
pub struct EcologicalPressures {
    pub binocular_vision: f32,
    pub peripheral_vision: f32,
    pub bite_force: f32,
    pub digestive_volume: f32,
    pub pursuit_speed: f32,
    pub camouflage: f32,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LimbRole {
    Support,
    Manipulation,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PostureBias {
    Defensive,
    Pursuit,
    Ambush,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MouthKind {
    CroppingGrinder,
    LeveragedJaw,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct MorphologyDependencies {
    pub bind_pose: BindPoseKind,
    pub hind_limb_role: LimbRole,
    pub fore_limb_role: LimbRole,
    pub pelvis_pitch_degrees: f32,
    pub shoulder_pitch_degrees: f32,
    pub spine_pitch_degrees: f32,
    pub spine_curve: f32,
    pub center_of_mass: [f32; 3],
    pub neck_compensation_degrees: f32,
    pub head_resting_angle_degrees: f32,
    pub gait: GaitRig,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BindPoseKind {
    QuadrupedReference,
    TPose,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct GaitRig {
    pub supporting_pairs: Vec<[String; 2]>,
    pub stride_bias: f32,
    pub balance_compensation: f32,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct Bone {
    pub name: String,
    pub parent: Option<usize>,
    pub start: [f32; 3],
    pub end: [f32; 3],
    pub load_bearing: bool,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ResolvedPart {
    Ellipsoid {
        role: String,
        center: [f32; 3],
        radii: [f32; 3],
        color: [u8; 3],
    },
    Limb {
        role: String,
        start: [f32; 3],
        end: [f32; 3],
        radius: f32,
        color: [u8; 3],
    },
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct ResolvedMorphology {
    pub locomotion: LocomotionKind,
    pub diet: DietKind,
    pub pressures: EcologicalPressures,
    pub posture_bias: PostureBias,
    pub mouth: MouthKind,
    pub terminal_component: ResolvedTerminalComponent,
    pub dependencies: MorphologyDependencies,
    pub bones: Vec<Bone>,
    pub parts: Vec<ResolvedPart>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct ResolvedTerminalComponent {
    pub recipe: String,
    pub component: String,
    pub version: u32,
    pub digit_count: u8,
    pub bones_per_digit: u8,
    pub supports_weight: bool,
    pub traction: String,
    pub grasping: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct CreatureState {
    pub generator_version: u32,
    pub recipe_name: String,
    pub seed: String,
    pub morphology: ResolvedMorphology,
}

pub fn ecological_pressures(diet: DietKind, variation: f32) -> EcologicalPressures {
    let variation = (variation - 0.5) * 0.12;
    match diet {
        DietKind::Herbivore => EcologicalPressures {
            binocular_vision: (0.28 + variation).clamp(0.0, 1.0),
            peripheral_vision: (0.86 - variation).clamp(0.0, 1.0),
            bite_force: (0.40 + variation * 0.5).clamp(0.0, 1.0),
            digestive_volume: (0.84 - variation * 0.4).clamp(0.0, 1.0),
            pursuit_speed: (0.30 + variation).clamp(0.0, 1.0),
            camouflage: (0.62 - variation * 0.5).clamp(0.0, 1.0),
        },
        DietKind::Carnivore => EcologicalPressures {
            binocular_vision: (0.86 + variation).clamp(0.0, 1.0),
            peripheral_vision: (0.38 - variation).clamp(0.0, 1.0),
            bite_force: (0.82 + variation * 0.5).clamp(0.0, 1.0),
            digestive_volume: (0.38 - variation * 0.4).clamp(0.0, 1.0),
            pursuit_speed: (0.78 + variation).clamp(0.0, 1.0),
            camouflage: (0.70 - variation * 0.5).clamp(0.0, 1.0),
        },
    }
}

pub fn compile_creature(
    seed: u64,
    _torso: TorsoKind,
    _head: HeadKind,
    terminal_component: &ComponentUse,
    _back: BackKind,
    locomotion: LocomotionKind,
    diet: DietKind,
) -> ResolvedMorphology {
    let variation = ((seed ^ (seed >> 32)) as u32) as f32 / u32::MAX as f32;
    let pressures = ecological_pressures(diet, variation);
    let posture_bias = match diet {
        DietKind::Herbivore => PostureBias::Defensive,
        DietKind::Carnivore if variation > 0.5 => PostureBias::Pursuit,
        DietKind::Carnivore => PostureBias::Ambush,
    };
    let mouth = match diet {
        DietKind::Herbivore => MouthKind::CroppingGrinder,
        DietKind::Carnivore => MouthKind::LeveragedJaw,
    };

    let terminal_component = resolve_terminal_component(seed, terminal_component);
    let dependencies = resolve_dependencies(locomotion, pressures);
    let (bones, parts) = build_body(locomotion, pressures, &dependencies, diet);
    ResolvedMorphology {
        locomotion,
        diet,
        pressures,
        posture_bias,
        mouth,
        terminal_component,
        dependencies,
        bones,
        parts,
    }
}

fn resolve_terminal_component(
    seed: u64,
    component_use: &ComponentUse,
) -> ResolvedTerminalComponent {
    let definition = component_use.definition.as_deref();
    let count = definition.map_or((3, 5), |value| {
        (
            value.structure.digits.count.min,
            value.structure.digits.count.max,
        )
    });
    let bones = definition.map_or((2, 3), |value| {
        (
            value.structure.digits.bones_per_digit.min,
            value.structure.digits.bones_per_digit.max,
        )
    });
    let digit_count = component_use
        .overrides
        .digits
        .as_ref()
        .and_then(|value| value.count)
        .unwrap_or_else(|| count.0 + (seed as u8 % (count.1 - count.0 + 1)));
    let grasping = component_use
        .overrides
        .constraints
        .as_ref()
        .and_then(|value| value.grasping)
        .or_else(|| definition.map(|value| value.constraints.grasping))
        .unwrap_or(CapabilityLevel::Low);
    let traction = definition
        .map(|value| value.constraints.traction)
        .unwrap_or(CapabilityLevel::High);
    ResolvedTerminalComponent {
        recipe: component_use.recipe.clone(),
        component: definition
            .map_or("paw", |value| value.component.as_str())
            .to_owned(),
        version: definition.map_or(1, |value| value.version),
        digit_count,
        bones_per_digit: bones.0 + ((seed >> 8) as u8 % (bones.1 - bones.0 + 1)),
        supports_weight: definition.is_none_or(|value| value.constraints.supports_weight),
        traction: format!("{traction:?}").to_lowercase(),
        grasping: format!("{grasping:?}").to_lowercase(),
    }
}

fn resolve_dependencies(
    locomotion: LocomotionKind,
    pressures: EcologicalPressures,
) -> MorphologyDependencies {
    let (fore_limb_role, pelvis_pitch, shoulder_pitch, spine_pitch) = match locomotion {
        LocomotionKind::Quadruped => (LimbRole::Support, 4.0, 2.0, 8.0),
        LocomotionKind::Biped => (LimbRole::Manipulation, 76.0, 52.0, 72.0),
    };
    let hind_limb_role = LimbRole::Support;
    let spine_curve = match locomotion {
        LocomotionKind::Quadruped => 0.16 + pressures.digestive_volume * 0.12,
        LocomotionKind::Biped => 0.30 + pressures.digestive_volume * 0.10,
    };
    let center_of_mass = match locomotion {
        LocomotionKind::Quadruped => [0.0, 1.08, -0.03 + pressures.digestive_volume * 0.08],
        LocomotionKind::Biped => [0.0, 1.35, -0.16 + pressures.digestive_volume * 0.10],
    };
    let neck_compensation = match locomotion {
        LocomotionKind::Quadruped => 6.0,
        LocomotionKind::Biped => -48.0,
    };
    let head_resting_angle = spine_pitch + neck_compensation;
    let gait = match locomotion {
        LocomotionKind::Quadruped => GaitRig {
            supporting_pairs: vec![
                ["left_fore".to_owned(), "right_hind".to_owned()],
                ["right_fore".to_owned(), "left_hind".to_owned()],
            ],
            stride_bias: 0.62 + pressures.pursuit_speed * 0.25,
            balance_compensation: 0.18,
        },
        LocomotionKind::Biped => GaitRig {
            supporting_pairs: vec![["left_hind".to_owned(), "right_hind".to_owned()]],
            stride_bias: 0.48 + pressures.pursuit_speed * 0.30,
            balance_compensation: 0.72,
        },
    };

    MorphologyDependencies {
        bind_pose: match locomotion {
            LocomotionKind::Quadruped => BindPoseKind::QuadrupedReference,
            LocomotionKind::Biped => BindPoseKind::TPose,
        },
        hind_limb_role,
        fore_limb_role,
        pelvis_pitch_degrees: pelvis_pitch,
        shoulder_pitch_degrees: shoulder_pitch,
        spine_pitch_degrees: spine_pitch,
        spine_curve,
        center_of_mass,
        neck_compensation_degrees: neck_compensation,
        head_resting_angle_degrees: head_resting_angle,
        gait,
    }
}

fn build_body(
    locomotion: LocomotionKind,
    pressures: EcologicalPressures,
    dependencies: &MorphologyDependencies,
    diet: DietKind,
) -> (Vec<Bone>, Vec<ResolvedPart>) {
    let body_color = match diet {
        DietKind::Herbivore => [72, 143, 62],
        DietKind::Carnivore => [55, 112, 70],
    };
    let belly = 0.72 + pressures.digestive_volume * 0.48;
    let torso_center = dependencies.center_of_mass;
    let (torso_radii, neck_base, neck_length, head_radii, bulb_offset) = match locomotion {
        LocomotionKind::Quadruped => (
            [0.76, belly * 0.62, 1.02],
            [0.0, torso_center[1] + 0.06, -0.72],
            0.72,
            [0.62, 0.46, 0.52],
            [0.0, 0.57, 0.12],
        ),
        LocomotionKind::Biped => (
            [0.68, belly * 0.78, 0.70],
            [0.0, torso_center[1] + 0.70, 0.0],
            0.68,
            [0.60, 0.46, 0.52],
            [0.0, 0.12, 0.66],
        ),
    };
    let head_angle = dependencies.head_resting_angle_degrees.to_radians();
    let head_center = [
        neck_base[0],
        neck_base[1] + head_angle.sin() * neck_length,
        neck_base[2] - head_angle.cos() * neck_length,
    ];
    let bulb_center = [
        torso_center[0] + bulb_offset[0],
        torso_center[1] + bulb_offset[1],
        torso_center[2] + bulb_offset[2],
    ];

    let mut bones = vec![
        bone(
            "pelvis",
            None,
            torso_center,
            [0.0, torso_center[1] + 0.2, 0.0],
            true,
        ),
        bone("spine", Some(0), torso_center, head_center, true),
        bone(
            "neck",
            Some(1),
            head_center,
            [head_center[0], head_center[1], head_center[2] + 0.2],
            false,
        ),
        bone(
            "jaw",
            Some(2),
            [head_center[0], head_center[1] - 0.10, head_center[2] - 0.08],
            [head_center[0], head_center[1] - 0.18, head_center[2] - 0.48],
            false,
        ),
    ];
    let mut parts = vec![
        ResolvedPart::Ellipsoid {
            role: "fruit_torso".to_owned(),
            center: torso_center,
            radii: torso_radii,
            color: body_color,
        },
        ResolvedPart::Ellipsoid {
            role: "frog_head".to_owned(),
            center: head_center,
            radii: head_radii,
            color: [83, 163, 76],
        },
        ResolvedPart::Ellipsoid {
            role: "back_bulb".to_owned(),
            center: bulb_center,
            radii: [0.48, 0.56, 0.48],
            color: [80, 116, 55],
        },
    ];

    let (fore_top_y, hind_top_y, ground_y, fore_z, hind_z) = match locomotion {
        LocomotionKind::Quadruped => (1.08, 1.04, -0.24, -0.58, 0.58),
        LocomotionKind::Biped => (1.62, 1.20, -0.30, -0.12, 0.12),
    };
    for (side_name, side) in [("left", -1.0_f32), ("right", 1.0_f32)] {
        let hind_start = [side * 0.48, hind_top_y, hind_z];
        let hind_end = [side * 0.38, ground_y, hind_z - 0.04];
        add_limb(
            &mut bones,
            &mut parts,
            format!("{side_name}_hind"),
            0,
            hind_start,
            hind_end,
            true,
            body_color,
        );

        let fore_start = [side * 0.50, fore_top_y, fore_z];
        let fore_end = match locomotion {
            LocomotionKind::Quadruped => [side * 0.42, ground_y, fore_z + 0.04],
            LocomotionKind::Biped => [side * 1.42, fore_top_y, -0.18],
        };
        add_limb(
            &mut bones,
            &mut parts,
            format!("{side_name}_fore"),
            1,
            fore_start,
            fore_end,
            locomotion == LocomotionKind::Quadruped,
            body_color,
        );
    }

    add_eyes(&mut parts, head_center, head_radii, pressures, diet);
    add_mouth(&mut parts, head_center, pressures, diet);
    (bones, parts)
}

fn add_eyes(
    parts: &mut Vec<ResolvedPart>,
    head: [f32; 3],
    radii: [f32; 3],
    pressures: EcologicalPressures,
    diet: DietKind,
) {
    let eye_angle = (70.0 - pressures.binocular_vision * 48.0).to_radians();
    let eye_color = match diet {
        DietKind::Herbivore => [29, 35, 18],
        DietKind::Carnivore => [16, 25, 14],
    };
    for side in [-1.0_f32, 1.0] {
        parts.push(ResolvedPart::Ellipsoid {
            role: "eye".to_owned(),
            center: [
                head[0] + side * eye_angle.sin() * (radii[0] + 0.05),
                head[1] + radii[1] * 0.35,
                head[2] - eye_angle.cos() * (radii[2] + 0.05),
            ],
            radii: [0.13, 0.15, 0.10],
            color: eye_color,
        });
    }
}

fn add_mouth(
    parts: &mut Vec<ResolvedPart>,
    head: [f32; 3],
    pressures: EcologicalPressures,
    diet: DietKind,
) {
    let jaw_depth = 0.16 + pressures.bite_force * 0.18;
    parts.push(ResolvedPart::Ellipsoid {
        role: match diet {
            DietKind::Herbivore => "cropping_mouth",
            DietKind::Carnivore => "leveraged_jaw",
        }
        .to_owned(),
        center: [head[0], head[1] - 0.16, head[2] - 0.48],
        radii: [0.34, 0.13, jaw_depth],
        color: [62, 75, 40],
    });
}

#[allow(clippy::too_many_arguments)]
fn add_limb(
    bones: &mut Vec<Bone>,
    parts: &mut Vec<ResolvedPart>,
    name: String,
    parent: usize,
    start: [f32; 3],
    end: [f32; 3],
    load_bearing: bool,
    color: [u8; 3],
) {
    bones.push(bone(&name, Some(parent), start, end, load_bearing));
    parts.push(ResolvedPart::Limb {
        role: name.clone(),
        start,
        end,
        radius: 0.16,
        color,
    });
    parts.push(ResolvedPart::Ellipsoid {
        role: format!("{name}_paw"),
        center: end,
        radii: [0.24, 0.12, 0.32],
        color: [61, 118, 52],
    });
}

fn bone(
    name: &str,
    parent: Option<usize>,
    start: [f32; 3],
    end: [f32; 3],
    load_bearing: bool,
) -> Bone {
    Bone {
        name: name.to_owned(),
        parent,
        start,
        end,
        load_bearing,
    }
}

#[allow(dead_code)]
fn _vec3(value: [f32; 3]) -> Vec3 {
    Vec3::new(value[0], value[1], value[2])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn compile(locomotion: LocomotionKind, diet: DietKind) -> ResolvedMorphology {
        compile_creature(
            42,
            TorsoKind::Fruit,
            HeadKind::Frog,
            &ComponentUse {
                recipe: "components/paw.yaml".to_owned(),
                overrides: Default::default(),
                definition: None,
            },
            BackKind::Bulb,
            locomotion,
            diet,
        )
    }

    #[test]
    fn diet_produces_weighted_ecological_pressures() {
        let herbivore = ecological_pressures(DietKind::Herbivore, 0.5);
        let carnivore = ecological_pressures(DietKind::Carnivore, 0.5);
        assert!(herbivore.peripheral_vision > carnivore.peripheral_vision);
        assert!(herbivore.digestive_volume > carnivore.digestive_volume);
        assert!(carnivore.binocular_vision > herbivore.binocular_vision);
        assert!(carnivore.bite_force > herbivore.bite_force);
        assert!(carnivore.pursuit_speed > herbivore.pursuit_speed);
    }

    #[test]
    fn locomotion_recomputes_dependency_chain() {
        let quadruped = compile(LocomotionKind::Quadruped, DietKind::Herbivore);
        let biped = compile(LocomotionKind::Biped, DietKind::Herbivore);
        assert_eq!(quadruped.dependencies.fore_limb_role, LimbRole::Support);
        assert_eq!(biped.dependencies.fore_limb_role, LimbRole::Manipulation);
        assert!(
            biped.dependencies.pelvis_pitch_degrees > quadruped.dependencies.pelvis_pitch_degrees
        );
        assert!(
            biped.dependencies.spine_pitch_degrees > quadruped.dependencies.spine_pitch_degrees
        );
        assert_ne!(
            biped.dependencies.center_of_mass,
            quadruped.dependencies.center_of_mass
        );
        assert_ne!(
            biped.dependencies.neck_compensation_degrees,
            quadruped.dependencies.neck_compensation_degrees
        );
        assert_ne!(biped.dependencies.gait, quadruped.dependencies.gait);
    }

    #[test]
    fn creature_state_round_trips_exactly() {
        let state = CreatureState {
            generator_version: GENERATOR_VERSION,
            recipe_name: "test creature".to_owned(),
            seed: "state-seed".to_owned(),
            morphology: compile(LocomotionKind::Quadruped, DietKind::Herbivore),
        };
        let yaml = yaml_serde::to_string(&state).expect("serialize state");
        let restored: CreatureState = yaml_serde::from_str(&yaml).expect("deserialize state");
        assert_eq!(restored, state);
    }
}
