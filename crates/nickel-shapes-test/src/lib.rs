use std::collections::HashMap;
use std::collections::hash_map::DefaultHasher;
use std::f32::consts::{PI, TAU};
use std::hash::{Hash, Hasher};
use std::path::Path;

use image::{ImageBuffer, Rgba, RgbaImage};
use serde::Deserialize;

mod morphology;
mod sculpt;

pub use morphology::{
    Bone, CreatureState, DietKind, LocomotionKind, ResolvedMorphology, ResolvedPart,
    compile_creature,
};
pub use sculpt::{AnatomicalFrame, SculptExperiment, SculptOperation, generate_sculpt_experiment};

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Vec3 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

impl Vec3 {
    pub const fn new(x: f32, y: f32, z: f32) -> Self {
        Self { x, y, z }
    }

    fn dot(self, other: Self) -> f32 {
        self.x * other.x + self.y * other.y + self.z * other.z
    }

    fn cross(self, other: Self) -> Self {
        Self::new(
            self.y * other.z - self.z * other.y,
            self.z * other.x - self.x * other.z,
            self.x * other.y - self.y * other.x,
        )
    }

    fn length(self) -> f32 {
        self.dot(self).sqrt()
    }

    fn normalized(self) -> Self {
        let length = self.length();
        if length > 1.0e-6 {
            self * (1.0 / length)
        } else {
            Self::new(0.0, 1.0, 0.0)
        }
    }
}

impl std::ops::Add for Vec3 {
    type Output = Self;

    fn add(self, rhs: Self) -> Self::Output {
        Self::new(self.x + rhs.x, self.y + rhs.y, self.z + rhs.z)
    }
}

impl std::ops::Sub for Vec3 {
    type Output = Self;

    fn sub(self, rhs: Self) -> Self::Output {
        Self::new(self.x - rhs.x, self.y - rhs.y, self.z - rhs.z)
    }
}

impl std::ops::Mul<f32> for Vec3 {
    type Output = Self;

    fn mul(self, rhs: f32) -> Self::Output {
        Self::new(self.x * rhs, self.y * rhs, self.z * rhs)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Vertex {
    pub position: Vec3,
    pub normal: Vec3,
    pub color: [u8; 3],
    pub surface_feature: [f32; 3],
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Mesh {
    pub vertices: Vec<Vertex>,
    pub indices: Vec<u32>,
}

impl Mesh {
    fn append(&mut self, mut other: Self) {
        let offset = self.vertices.len() as u32;
        self.vertices.append(&mut other.vertices);
        self.indices
            .extend(other.indices.into_iter().map(|index| index + offset));
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Lod {
    Distant,
    Gameplay,
    Close,
    Inspection,
}

impl Lod {
    pub const ALL: [Self; 4] = [Self::Distant, Self::Gameplay, Self::Close, Self::Inspection];

    pub fn from_index(index: u8) -> Option<Self> {
        Self::ALL.get(index as usize).copied()
    }

    pub const fn index(self) -> u8 {
        match self {
            Self::Distant => 0,
            Self::Gameplay => 1,
            Self::Close => 2,
            Self::Inspection => 3,
        }
    }

    const fn body_segments(self) -> (usize, usize) {
        match self {
            Self::Distant => (8, 12),
            Self::Gameplay => (16, 24),
            Self::Close => (28, 42),
            Self::Inspection => (40, 64),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Hash, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct OrganismRecipe {
    pub name: String,
    #[serde(default = "default_seed")]
    pub seed: String,
    pub root: NodeRecipe,
    pub motion: Option<MotionKind>,
}

#[derive(Clone, Copy, Debug, Deserialize, Hash, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum SkinKind {
    Apple,
}

#[derive(Clone, Debug, Deserialize, Hash, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum NodeRecipe {
    Sphere {
        skin: SkinKind,
        #[serde(default)]
        children: Vec<NodeRecipe>,
    },
    Branch {
        #[serde(default)]
        children: Vec<NodeRecipe>,
    },
    Leaf,
    GrapeCluster,
    Creature {
        torso: TorsoKind,
        head: HeadKind,
        terminal_component: ComponentUse,
        back: BackKind,
        locomotion: LocomotionKind,
        diet: DietKind,
    },
}

#[derive(Clone, Copy, Debug, Deserialize, Hash, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum TorsoKind {
    Fruit,
}

#[derive(Clone, Copy, Debug, Deserialize, Hash, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum HeadKind {
    Frog,
}

#[derive(Clone, Debug, Deserialize, Hash, PartialEq)]
#[serde(rename_all = "snake_case")]
pub struct ComponentUse {
    pub recipe: String,
    #[serde(default)]
    pub overrides: ComponentOverrides,
    #[serde(skip)]
    pub definition: Option<Box<AnatomicalComponent>>,
}

#[derive(Clone, Debug, Default, Deserialize, Hash, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ComponentOverrides {
    pub digits: Option<DigitOverrides>,
    pub constraints: Option<ConstraintOverrides>,
}

#[derive(Clone, Debug, Deserialize, Hash, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct DigitOverrides {
    pub count: Option<u8>,
}

#[derive(Clone, Debug, Deserialize, Hash, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ConstraintOverrides {
    pub grasping: Option<CapabilityLevel>,
}

#[derive(Clone, Copy, Debug, Deserialize, Hash, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityLevel {
    Low,
    Moderate,
    High,
}

#[derive(Clone, Debug, Deserialize, Hash, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct AnatomicalComponent {
    pub component: String,
    pub version: u32,
    pub structure: ComponentStructure,
    pub constraints: ComponentConstraints,
}

#[derive(Clone, Debug, Deserialize, Hash, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ComponentStructure {
    pub palm: PalmStructure,
    pub digits: DigitStructure,
    pub pads: PadStructure,
}

#[derive(Clone, Debug, Deserialize, Hash, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PalmStructure {
    pub kind: String,
}

#[derive(Clone, Debug, Deserialize, Hash, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct DigitStructure {
    pub count: IntegerRange,
    pub bones_per_digit: IntegerRange,
    pub spread: CapabilityLevel,
}

#[derive(Clone, Debug, Deserialize, Hash, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct IntegerRange {
    pub min: u8,
    pub max: u8,
}

#[derive(Clone, Debug, Deserialize, Hash, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PadStructure {
    pub material: String,
}

#[derive(Clone, Debug, Deserialize, Hash, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ComponentConstraints {
    pub supports_weight: bool,
    pub traction: CapabilityLevel,
    pub grasping: CapabilityLevel,
}

#[derive(Clone, Copy, Debug, Deserialize, Hash, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum BackKind {
    Bulb,
}

#[derive(Clone, Copy, Debug, Deserialize, Hash, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum MotionKind {
    Walking,
}

fn default_seed() -> String {
    "default".to_owned()
}

impl OrganismRecipe {
    pub fn from_yaml(source: &str) -> Result<Self, String> {
        yaml_serde::from_str(source).map_err(|error| format!("invalid shape YAML: {error}"))
    }

    pub fn with_seed(mut self, seed: impl Into<String>) -> Self {
        self.seed = seed.into();
        self
    }

    pub fn resolve_components(&mut self, recipe_directory: &Path) -> Result<(), String> {
        resolve_node_components(&mut self.root, recipe_directory)
    }

    fn fingerprint(&self) -> u64 {
        let mut hasher = DefaultHasher::new();
        self.hash(&mut hasher);
        hasher.finish()
    }
}

fn resolve_node_components(node: &mut NodeRecipe, recipe_directory: &Path) -> Result<(), String> {
    match node {
        NodeRecipe::Sphere { children, .. } | NodeRecipe::Branch { children } => {
            for child in children {
                resolve_node_components(child, recipe_directory)?;
            }
        }
        NodeRecipe::Creature {
            terminal_component, ..
        } => {
            let path = recipe_directory.join(&terminal_component.recipe);
            let source = std::fs::read_to_string(&path)
                .map_err(|error| format!("failed to read component {}: {error}", path.display()))?;
            terminal_component.definition = Some(Box::new(
                yaml_serde::from_str(&source)
                    .map_err(|error| format!("invalid component {}: {error}", path.display()))?,
            ));
        }
        NodeRecipe::Leaf | NodeRecipe::GrapeCluster => {}
    }
    Ok(())
}

#[derive(Default)]
pub struct OrganismMeshCache {
    meshes: HashMap<(u64, Lod), Mesh>,
}

impl OrganismMeshCache {
    pub fn get(&mut self, recipe: &OrganismRecipe, lod: Lod) -> &Mesh {
        self.meshes
            .entry((recipe.fingerprint(), lod))
            .or_insert_with(|| generate_shape(recipe, lod))
    }

    pub fn len(&self) -> usize {
        self.meshes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.meshes.is_empty()
    }
}

pub fn generate_shape(recipe: &OrganismRecipe, lod: Lod) -> Mesh {
    let seed = string_seed(&recipe.seed);
    generate_node(&recipe.root, lod, seed, Vec3::default())
}

pub fn creature_state(recipe: &OrganismRecipe) -> Option<CreatureState> {
    let NodeRecipe::Creature {
        torso,
        head,
        terminal_component,
        back,
        locomotion,
        diet,
    } = &recipe.root
    else {
        return None;
    };
    Some(CreatureState {
        generator_version: morphology::GENERATOR_VERSION,
        recipe_name: recipe.name.clone(),
        seed: recipe.seed.clone(),
        morphology: compile_creature(
            string_seed(&recipe.seed),
            *torso,
            *head,
            terminal_component,
            *back,
            *locomotion,
            *diet,
        ),
    })
}

fn generate_body(
    skin: SkinKind,
    children: &[NodeRecipe],
    lod: Lod,
    seed: u64,
    origin: Vec3,
) -> Mesh {
    let (rings, sides) = lod.body_segments();
    let mut vertices = Vec::with_capacity((rings + 1) * sides);
    let color_phase = hash_unit(seed) * TAU;
    let width = random_range(seed, 10, 0.92, 1.08);
    let height = random_range(seed, 11, 0.98, 1.13);
    let has_top_attachment = children
        .iter()
        .any(|child| matches!(child, NodeRecipe::Branch { .. }));
    let attachment_dimple = if has_top_attachment {
        random_range(seed, 12, 0.12, 0.20)
    } else {
        0.0
    };
    let lobe_count = random_range(seed, 13, 4.0, 7.0).round();
    let lobe_strength = random_range(seed, 14, 0.018, 0.048);
    let (tone_a, tone_b) = skin_colors(skin, seed);
    let stripe_count = random_range(seed, 15, 4.0, 9.0).round();

    for ring in 0..=rings {
        let v = ring as f32 / rings as f32;
        let polar = v * PI;
        let sin_polar = polar.sin();
        let y_base = polar.cos() * height;
        let top = smoothstep(0.72, 1.0, y_base.max(0.0) / height);
        let y = y_base - attachment_dimple * top;

        for side in 0..sides {
            let u = side as f32 / sides as f32;
            let azimuth = u * TAU;
            let lobe = 1.0
                + lobe_strength
                    * (azimuth * lobe_count + color_phase * 0.12).cos()
                    * sin_polar.powi(2);
            let shoulder = 1.0 + 0.12 * smoothstep(0.15, 0.55, v) * (1.0 - v);
            let radius = width * sin_polar * lobe * shoulder;
            let position = origin + Vec3::new(radius * azimuth.cos(), y, radius * azimuth.sin());

            let stripe = (azimuth * stripe_count + color_phase).sin() * 0.5 + 0.5;
            let color = mix_color(tone_a, tone_b, stripe);
            vertices.push(Vertex {
                position,
                normal: Vec3::default(),
                color,
                surface_feature: [2.0, 2.0, 2.0],
            });
        }
    }

    let mut indices = Vec::with_capacity(rings * sides * 6);
    for ring in 0..rings {
        for side in 0..sides {
            let next = (side + 1) % sides;
            let a = (ring * sides + side) as u32;
            let b = (ring * sides + next) as u32;
            let c = ((ring + 1) * sides + side) as u32;
            let d = ((ring + 1) * sides + next) as u32;
            indices.extend_from_slice(&[a, c, b, b, c, d]);
        }
    }

    let mut mesh = Mesh { vertices, indices };
    recompute_normals(&mut mesh);
    mesh
}

fn generate_node(node: &NodeRecipe, lod: Lod, seed: u64, origin: Vec3) -> Mesh {
    match node {
        NodeRecipe::Sphere { skin, children } => {
            let mut mesh = generate_body(*skin, children, lod, seed, origin);
            for (index, child) in children.iter().enumerate() {
                mesh.append(generate_node(
                    child,
                    lod,
                    mixed_seed(seed, index as u64 + 1),
                    origin + Vec3::new(0.0, 0.84, 0.0),
                ));
            }
            mesh
        }
        NodeRecipe::Branch { children } => generate_branch(children, lod, seed, origin),
        NodeRecipe::Leaf => generate_leaf(lod, seed, origin),
        NodeRecipe::GrapeCluster => generate_grape_cluster(lod, seed, origin),
        NodeRecipe::Creature {
            torso,
            head,
            terminal_component,
            back,
            locomotion,
            diet,
        } => {
            let morphology = compile_creature(
                seed,
                *torso,
                *head,
                terminal_component,
                *back,
                *locomotion,
                *diet,
            );
            generate_morphology_mesh(&morphology, lod, origin)
        }
    }
}

fn generate_branch(children: &[NodeRecipe], lod: Lod, seed: u64, origin: Vec3) -> Mesh {
    let sides = match lod {
        Lod::Distant => 5,
        Lod::Gameplay => 7,
        Lod::Close => 9,
        Lod::Inspection => 12,
    };
    let rings = sides;
    let mut mesh = Mesh::default();
    let direction = if hash_unit(mixed_seed(seed, 20)) > 0.5 {
        1.0
    } else {
        -1.0
    };
    let control_points = [
        origin,
        origin + Vec3::new(0.02 * direction, 0.28, 0.0),
        origin
            + Vec3::new(
                random_range(seed, 21, 0.10, 0.20) * direction,
                0.46,
                random_range(seed, 22, -0.07, 0.07),
            ),
        origin
            + Vec3::new(
                random_range(seed, 23, 0.07, 0.17) * direction,
                random_range(seed, 24, 0.58, 0.70),
                random_range(seed, 25, -0.10, 0.10),
            ),
    ];

    for ring in 0..=rings {
        let t = ring as f32 / rings as f32;
        let center = cubic_bezier(
            control_points[0],
            control_points[1],
            control_points[2],
            control_points[3],
            t,
        );
        let radius = random_range(seed, 26, 0.045, 0.065) * (1.0 - 0.52 * t);
        for side in 0..sides {
            let angle = side as f32 / sides as f32 * TAU;
            mesh.vertices.push(Vertex {
                position: center + Vec3::new(angle.cos() * radius, 0.0, angle.sin() * radius),
                normal: Vec3::default(),
                color: [91, 58, 32],
                surface_feature: [2.0, 2.0, 2.0],
            });
        }
    }
    connect_grid(&mut mesh.indices, rings, sides);
    recompute_normals(&mut mesh);
    for (index, child) in children.iter().enumerate() {
        let child_seed = mixed_seed(seed, index as u64 + 100);
        let spread = children.len().max(1) as f32;
        let t = (0.48 + index as f32 * 0.38 / spread).clamp(0.0, 0.92);
        let child_origin = cubic_bezier(
            control_points[0],
            control_points[1],
            control_points[2],
            control_points[3],
            t,
        );
        mesh.append(generate_node(child, lod, child_seed, child_origin));
    }
    mesh
}

fn generate_leaf(lod: Lod, seed: u64, origin: Vec3) -> Mesh {
    let length_segments = match lod {
        Lod::Distant => 4,
        Lod::Gameplay => 7,
        Lod::Close => 11,
        Lod::Inspection => 16,
    };
    let width_segments = match lod {
        Lod::Distant => 2,
        Lod::Gameplay => 4,
        Lod::Close => 6,
        Lod::Inspection => 8,
    };
    let side = if hash_unit(mixed_seed(seed, 30)) > 0.5 {
        1.0
    } else {
        -1.0
    };
    let direction = Vec3::new(
        random_range(seed, 31, 0.30, 0.55) * side,
        random_range(seed, 32, 0.12, 0.26),
        random_range(seed, 33, -0.20, 0.20),
    )
    .normalized();
    let across = Vec3::new(-direction.z, 0.0, direction.x).normalized();
    let length = random_range(seed, 34, 0.55, 0.78);
    let width = random_range(seed, 35, 0.15, 0.23);
    let curl = random_range(seed, 36, 0.035, 0.075);
    let color = if hash_unit(mixed_seed(seed, 37)) > 0.5 {
        [63, 137, 60]
    } else {
        [78, 156, 68]
    };
    let mut mesh = Mesh::default();

    for along_index in 0..=length_segments {
        let t = along_index as f32 / length_segments as f32;
        let center = origin
            + direction * (length * t)
            + Vec3::new(0.0, -0.22 * t * t + 0.10 * (t * PI).sin(), 0.0);
        let half_width = width * (t * PI).sin();
        for width_index in 0..=width_segments {
            let s = width_index as f32 / width_segments as f32 * 2.0 - 1.0;
            let curl = curl * s * s + 0.035 * (t * PI).sin();
            mesh.vertices.push(Vertex {
                position: center + across * (half_width * s) + Vec3::new(0.0, curl, 0.0),
                normal: Vec3::default(),
                color,
                surface_feature: [2.0, 2.0, 2.0],
            });
        }
    }

    connect_grid_with_stride(
        &mut mesh.indices,
        length_segments,
        width_segments,
        width_segments + 1,
    );
    recompute_normals(&mut mesh);
    mesh
}

fn generate_grape_cluster(lod: Lod, seed: u64, origin: Vec3) -> Mesh {
    let count = random_range(seed, 40, 10.0, 17.0).round() as usize;
    let mut mesh = Mesh::default();
    for index in 0..count {
        let row = (index as f32 / 4.0).floor();
        let angle = index as f32 * 2.399_963_1 + hash_unit(seed) * TAU;
        let spread = (0.22 - row * 0.025).max(0.04);
        let center = origin
            + Vec3::new(
                angle.cos() * spread,
                -0.12 - row * 0.15,
                angle.sin() * spread,
            );
        let radius = random_range(seed, index as u64 + 50, 0.10, 0.14);
        let color = if hash_unit(mixed_seed(seed, index as u64 + 70)) > 0.5 {
            [84, 37, 128]
        } else {
            [116, 62, 157]
        };
        mesh.append(generate_sphere(lod, center, radius, color));
    }
    mesh
}

fn generate_sphere(lod: Lod, center: Vec3, radius: f32, color: [u8; 3]) -> Mesh {
    let (body_rings, body_sides) = lod.body_segments();
    let rings = (body_rings / 3).max(4);
    let sides = (body_sides / 3).max(6);
    let mut mesh = Mesh::default();
    for ring in 0..=rings {
        let polar = ring as f32 / rings as f32 * PI;
        for side in 0..sides {
            let azimuth = side as f32 / sides as f32 * TAU;
            let normal = Vec3::new(
                polar.sin() * azimuth.cos(),
                polar.cos(),
                polar.sin() * azimuth.sin(),
            );
            mesh.vertices.push(Vertex {
                position: center + normal * radius,
                normal,
                color,
                surface_feature: [2.0, 2.0, 2.0],
            });
        }
    }
    connect_grid(&mut mesh.indices, rings, sides);
    mesh
}

fn generate_morphology_mesh(morphology: &ResolvedMorphology, lod: Lod, origin: Vec3) -> Mesh {
    let mut mesh = generate_skin_mesh(morphology, lod, origin);
    for part in &morphology.parts {
        match part {
            ResolvedPart::Ellipsoid {
                role,
                center,
                radii,
                color,
                ..
            } if role == "eye" => mesh.append(generate_eye_component(
                lod,
                origin + array_vec(*center),
                array_vec(*radii),
                *color,
            )),
            ResolvedPart::Ellipsoid { .. } | ResolvedPart::Limb { .. } => {}
        }
    }
    mesh
}

fn generate_eye_component(lod: Lod, center: Vec3, radii: Vec3, iris_color: [u8; 3]) -> Mesh {
    let mut mesh = generate_ellipsoid(lod, center, radii, [178, 188, 132]);
    let iris_center = center + Vec3::new(0.0, 0.0, -radii.z * 0.86);
    mesh.append(generate_ellipsoid(
        lod,
        iris_center,
        Vec3::new(radii.x * 0.62, radii.y * 0.68, radii.z * 0.28),
        iris_color,
    ));
    mesh.append(generate_ellipsoid(
        lod,
        iris_center + Vec3::new(0.0, 0.0, -radii.z * 0.18),
        Vec3::new(radii.x * 0.22, radii.y * 0.48, radii.z * 0.16),
        [8, 12, 7],
    ));
    mesh
}

struct SkinField<'a> {
    morphology: &'a ResolvedMorphology,
}

impl SkinField<'_> {
    fn sample(&self, point: Vec3) -> f32 {
        let mut distance = f32::INFINITY;
        for bone in &self.morphology.bones {
            let radius = match bone.name.as_str() {
                "pelvis" => 0.50 + self.morphology.pressures.digestive_volume * 0.18,
                "spine" => 0.48 + self.morphology.pressures.digestive_volume * 0.22,
                "neck" => 0.46,
                "jaw" => 0.30,
                name if name.ends_with("_hind") => 0.19,
                name if name.ends_with("_fore") => 0.17,
                _ => 0.16,
            };
            distance = smooth_min(
                distance,
                capsule_distance(point, array_vec(bone.start), array_vec(bone.end), radius),
                0.22,
            );
        }
        for part in &self.morphology.parts {
            if let ResolvedPart::Ellipsoid {
                role,
                center,
                radii,
                ..
            } = part
                && is_skin_volume(role)
            {
                distance = smooth_min(
                    distance,
                    ellipsoid_distance(point, array_vec(*center), array_vec(*radii)),
                    if role == "back_bulb" { 0.14 } else { 0.28 },
                );
            }
        }
        for (start, end, radius) in paw_digit_capsules(self.morphology) {
            distance = smooth_min(distance, capsule_distance(point, start, end, radius), 0.08);
        }
        distance
    }

    fn normal(&self, point: Vec3) -> Vec3 {
        let epsilon = 0.008;
        Vec3::new(
            self.sample(point + Vec3::new(epsilon, 0.0, 0.0))
                - self.sample(point - Vec3::new(epsilon, 0.0, 0.0)),
            self.sample(point + Vec3::new(0.0, epsilon, 0.0))
                - self.sample(point - Vec3::new(0.0, epsilon, 0.0)),
            self.sample(point + Vec3::new(0.0, 0.0, epsilon))
                - self.sample(point - Vec3::new(0.0, 0.0, epsilon)),
        )
        .normalized()
    }

    fn color(&self, point: Vec3) -> [u8; 3] {
        let mouth = self.mouth_projection(point);
        if mouth[2] < 1.0 && mouth[0] * mouth[0] + mouth[1] * mouth[1] < 1.0 {
            return [31, 20, 18];
        }
        for part in &self.morphology.parts {
            if let ResolvedPart::Ellipsoid {
                role,
                center,
                radii,
                color,
            } = part
                && (role == "back_bulb" || role.ends_with("_paw"))
                && ellipsoid_distance(point, array_vec(*center), array_vec(*radii)) < 0.08
            {
                return *color;
            }
        }
        match self.morphology.diet {
            DietKind::Herbivore => [91, 174, 82],
            DietKind::Carnivore => [65, 132, 80],
        }
    }

    fn mouth_projection(&self, point: Vec3) -> [f32; 3] {
        for part in &self.morphology.parts {
            let ResolvedPart::Ellipsoid {
                role,
                center,
                radii,
                ..
            } = part
            else {
                continue;
            };
            if role != "cropping_mouth" && role != "leveraged_jaw" {
                continue;
            }
            let center = array_vec(*center);
            let radii = array_vec(*radii);
            let relative = point - center;
            let horizontal = relative.x / radii.x.max(1.0e-4);
            let vertical = relative.y / (radii.y * 0.42).max(1.0e-4);
            let depth = relative.z.abs() / (radii.z * 1.65).max(1.0e-4);
            return [horizontal, vertical, depth];
        }
        [2.0, 2.0, 2.0]
    }
}

fn is_skin_volume(role: &str) -> bool {
    role == "fruit_torso" || role == "frog_head" || role == "back_bulb" || role.ends_with("_paw")
}

fn paw_digit_capsules(morphology: &ResolvedMorphology) -> Vec<(Vec3, Vec3, f32)> {
    let count = usize::from(morphology.terminal_component.digit_count.max(1));
    let mut digits = Vec::new();
    for part in &morphology.parts {
        let ResolvedPart::Ellipsoid {
            role,
            center,
            radii,
            ..
        } = part
        else {
            continue;
        };
        if !role.ends_with("_paw") {
            continue;
        }
        let center = array_vec(*center);
        let radii = array_vec(*radii);
        for index in 0..count {
            let across = if count == 1 {
                0.0
            } else {
                index as f32 / (count - 1) as f32 * 2.0 - 1.0
            };
            let start =
                center + Vec3::new(across * radii.x * 0.66, -radii.y * 0.04, -radii.z * 0.44);
            let end = start + Vec3::new(across * 0.045, -0.015, -(0.16 + radii.z * 0.42));
            digits.push((start, end, 0.055 + radii.x * 0.08));
        }
    }
    digits
}

fn generate_skin_mesh(morphology: &ResolvedMorphology, lod: Lod, origin: Vec3) -> Mesh {
    let resolution = match lod {
        Lod::Distant => 16,
        Lod::Gameplay => 24,
        Lod::Close => 34,
        Lod::Inspection => 44,
    };
    let field = SkinField { morphology };
    let (minimum, maximum) = skeleton_bounds(morphology);
    let size = maximum - minimum;
    let step = Vec3::new(
        size.x / resolution as f32,
        size.y / resolution as f32,
        size.z / resolution as f32,
    );
    let mut mesh = Mesh::default();
    let tetrahedra = [
        [0, 5, 1, 6],
        [0, 1, 2, 6],
        [0, 2, 3, 6],
        [0, 3, 7, 6],
        [0, 7, 4, 6],
        [0, 4, 5, 6],
    ];
    for z in 0..resolution {
        for y in 0..resolution {
            for x in 0..resolution {
                let base =
                    minimum + Vec3::new(x as f32 * step.x, y as f32 * step.y, z as f32 * step.z);
                let points = [
                    base,
                    base + Vec3::new(step.x, 0.0, 0.0),
                    base + Vec3::new(step.x, step.y, 0.0),
                    base + Vec3::new(0.0, step.y, 0.0),
                    base + Vec3::new(0.0, 0.0, step.z),
                    base + Vec3::new(step.x, 0.0, step.z),
                    base + Vec3::new(step.x, step.y, step.z),
                    base + Vec3::new(0.0, step.y, step.z),
                ];
                let values = points.map(|point| field.sample(point));
                if values.iter().all(|value| *value > 0.0)
                    || values.iter().all(|value| *value <= 0.0)
                {
                    continue;
                }
                for tetrahedron in tetrahedra {
                    polygonize_tetrahedron(
                        &mut mesh,
                        &field,
                        origin,
                        tetrahedron.map(|index| points[index]),
                        tetrahedron.map(|index| values[index]),
                    );
                }
            }
        }
    }
    mesh
}

fn polygonize_tetrahedron(
    mesh: &mut Mesh,
    field: &SkinField<'_>,
    origin: Vec3,
    points: [Vec3; 4],
    values: [f32; 4],
) {
    let inside: Vec<_> = (0..4).filter(|index| values[*index] <= 0.0).collect();
    let outside: Vec<_> = (0..4).filter(|index| values[*index] > 0.0).collect();
    match inside.len() {
        1 => {
            let center = inside[0];
            let triangle: [Vec3; 3] = std::array::from_fn(|index| {
                skin_intersection(points, values, center, outside[index])
            });
            push_skin_triangle(mesh, field, origin, triangle[0], triangle[1], triangle[2]);
        }
        3 => {
            let center = outside[0];
            let triangle: [Vec3; 3] = std::array::from_fn(|index| {
                skin_intersection(points, values, center, inside[index])
            });
            push_skin_triangle(mesh, field, origin, triangle[0], triangle[1], triangle[2]);
        }
        2 => {
            let a = skin_intersection(points, values, inside[0], outside[0]);
            let b = skin_intersection(points, values, inside[1], outside[0]);
            let c = skin_intersection(points, values, inside[0], outside[1]);
            let d = skin_intersection(points, values, inside[1], outside[1]);
            push_skin_triangle(mesh, field, origin, a, b, c);
            push_skin_triangle(mesh, field, origin, c, b, d);
        }
        0 | 4 => {}
        _ => unreachable!("a tetrahedron has four vertices"),
    }
}

fn skin_intersection(points: [Vec3; 4], values: [f32; 4], a: usize, b: usize) -> Vec3 {
    let amount = values[a] / (values[a] - values[b]);
    points[a] + (points[b] - points[a]) * amount
}

fn push_skin_triangle(
    mesh: &mut Mesh,
    field: &SkinField<'_>,
    origin: Vec3,
    mut a: Vec3,
    mut b: Vec3,
    c: Vec3,
) {
    let average = (a + b + c) * (1.0 / 3.0);
    let outward = field.normal(average);
    if (b - a).cross(c - a).dot(outward) < 0.0 {
        std::mem::swap(&mut a, &mut b);
    }
    let start = mesh.vertices.len() as u32;
    for point in [a, b, c] {
        mesh.vertices.push(Vertex {
            position: origin + point,
            normal: field.normal(point),
            color: field.color(point),
            surface_feature: field.mouth_projection(point),
        });
    }
    mesh.indices
        .extend_from_slice(&[start, start + 1, start + 2]);
}

fn skeleton_bounds(morphology: &ResolvedMorphology) -> (Vec3, Vec3) {
    let mut minimum = Vec3::new(f32::INFINITY, f32::INFINITY, f32::INFINITY);
    let mut maximum = Vec3::new(f32::NEG_INFINITY, f32::NEG_INFINITY, f32::NEG_INFINITY);
    for bone in &morphology.bones {
        for point in [array_vec(bone.start), array_vec(bone.end)] {
            minimum.x = minimum.x.min(point.x);
            minimum.y = minimum.y.min(point.y);
            minimum.z = minimum.z.min(point.z);
            maximum.x = maximum.x.max(point.x);
            maximum.y = maximum.y.max(point.y);
            maximum.z = maximum.z.max(point.z);
        }
    }
    for part in &morphology.parts {
        if let ResolvedPart::Ellipsoid {
            role,
            center,
            radii,
            ..
        } = part
            && is_skin_volume(role)
        {
            let center = array_vec(*center);
            let radii = array_vec(*radii);
            minimum.x = minimum.x.min(center.x - radii.x);
            minimum.y = minimum.y.min(center.y - radii.y);
            minimum.z = minimum.z.min(center.z - radii.z);
            maximum.x = maximum.x.max(center.x + radii.x);
            maximum.y = maximum.y.max(center.y + radii.y);
            maximum.z = maximum.z.max(center.z + radii.z);
        }
    }
    for (start, end, radius) in paw_digit_capsules(morphology) {
        for point in [start, end] {
            minimum.x = minimum.x.min(point.x - radius);
            minimum.y = minimum.y.min(point.y - radius);
            minimum.z = minimum.z.min(point.z - radius);
            maximum.x = maximum.x.max(point.x + radius);
            maximum.y = maximum.y.max(point.y + radius);
            maximum.z = maximum.z.max(point.z + radius);
        }
    }
    let padding = Vec3::new(0.34, 0.34, 0.34);
    (minimum - padding, maximum + padding)
}

fn capsule_distance(point: Vec3, start: Vec3, end: Vec3, radius: f32) -> f32 {
    let segment = end - start;
    let amount = ((point - start).dot(segment) / segment.dot(segment).max(1.0e-6)).clamp(0.0, 1.0);
    (point - (start + segment * amount)).length() - radius
}

fn ellipsoid_distance(point: Vec3, center: Vec3, radii: Vec3) -> f32 {
    let relative = point - center;
    let normalized = Vec3::new(
        relative.x / radii.x,
        relative.y / radii.y,
        relative.z / radii.z,
    );
    (normalized.length() - 1.0) * radii.x.min(radii.y).min(radii.z)
}

fn smooth_min(a: f32, b: f32, blend: f32) -> f32 {
    if !a.is_finite() {
        return b;
    }
    let amount = (0.5 + 0.5 * (b - a) / blend).clamp(0.0, 1.0);
    b * (1.0 - amount) + a * amount - blend * amount * (1.0 - amount)
}

fn generate_ellipsoid(lod: Lod, center: Vec3, radii: Vec3, color: [u8; 3]) -> Mesh {
    let mut mesh = generate_sphere(lod, Vec3::default(), 1.0, color);
    for vertex in &mut mesh.vertices {
        vertex.position = center
            + Vec3::new(
                vertex.position.x * radii.x,
                vertex.position.y * radii.y,
                vertex.position.z * radii.z,
            );
        vertex.normal = Vec3::new(
            vertex.normal.x / radii.x.max(1.0e-4),
            vertex.normal.y / radii.y.max(1.0e-4),
            vertex.normal.z / radii.z.max(1.0e-4),
        )
        .normalized();
    }
    mesh
}

fn array_vec(value: [f32; 3]) -> Vec3 {
    Vec3::new(value[0], value[1], value[2])
}

fn skin_colors(skin: SkinKind, seed: u64) -> ([u8; 3], [u8; 3]) {
    match skin {
        SkinKind::Apple => {
            let palettes = [
                ([116, 10, 24], [224, 49, 51]),
                ([69, 111, 19], [163, 205, 57]),
                ([158, 91, 5], [244, 190, 42]),
                ([105, 20, 35], [210, 120, 45]),
            ];
            palettes[(mixed_seed(seed, 90) as usize) % palettes.len()]
        }
    }
}

fn connect_grid(indices: &mut Vec<u32>, rows: usize, sides: usize) {
    for row in 0..rows {
        for side in 0..sides {
            let next = (side + 1) % sides;
            let a = (row * sides + side) as u32;
            let b = (row * sides + next) as u32;
            let c = ((row + 1) * sides + side) as u32;
            let d = ((row + 1) * sides + next) as u32;
            indices.extend_from_slice(&[a, c, b, b, c, d]);
        }
    }
}

fn connect_grid_with_stride(indices: &mut Vec<u32>, rows: usize, columns: usize, stride: usize) {
    for row in 0..rows {
        for column in 0..columns {
            let a = (row * stride + column) as u32;
            let b = a + 1;
            let c = ((row + 1) * stride + column) as u32;
            let d = c + 1;
            indices.extend_from_slice(&[a, c, b, b, c, d]);
        }
    }
}

fn recompute_normals(mesh: &mut Mesh) {
    for vertex in &mut mesh.vertices {
        vertex.normal = Vec3::default();
    }
    for triangle in mesh.indices.chunks_exact(3) {
        let a = mesh.vertices[triangle[0] as usize].position;
        let b = mesh.vertices[triangle[1] as usize].position;
        let c = mesh.vertices[triangle[2] as usize].position;
        let normal = (b - a).cross(c - a);
        for index in triangle {
            mesh.vertices[*index as usize].normal = mesh.vertices[*index as usize].normal + normal;
        }
    }
    for vertex in &mut mesh.vertices {
        vertex.normal = vertex.normal.normalized();
    }
}

fn cubic_bezier(a: Vec3, b: Vec3, c: Vec3, d: Vec3, t: f32) -> Vec3 {
    let inverse = 1.0 - t;
    a * inverse.powi(3)
        + b * (3.0 * inverse.powi(2) * t)
        + c * (3.0 * inverse * t * t)
        + d * t.powi(3)
}

fn smoothstep(start: f32, end: f32, value: f32) -> f32 {
    let t = ((value - start) / (end - start)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn string_seed(value: &str) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for byte in value.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

fn mixed_seed(seed: u64, salt: u64) -> u64 {
    let mut value = seed ^ salt.wrapping_mul(0x9e37_79b9_7f4a_7c15);
    value ^= value >> 30;
    value = value.wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value ^= value >> 27;
    value = value.wrapping_mul(0x94d0_49bb_1331_11eb);
    value ^ (value >> 31)
}

fn random_range(seed: u64, salt: u64, minimum: f32, maximum: f32) -> f32 {
    minimum + (maximum - minimum) * hash_unit(mixed_seed(seed, salt))
}

fn hash_unit(mut value: u64) -> f32 {
    value ^= value >> 30;
    value = value.wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value ^= value >> 27;
    value = value.wrapping_mul(0x94d0_49bb_1331_11eb);
    value ^= value >> 31;
    (value as u32) as f32 / u32::MAX as f32
}

fn mix_color(a: [u8; 3], b: [u8; 3], amount: f32) -> [u8; 3] {
    std::array::from_fn(|index| (a[index] as f32 * (1.0 - amount) + b[index] as f32 * amount) as u8)
}

#[derive(Clone, Copy)]
struct ScreenVertex {
    x: f32,
    y: f32,
    depth: f32,
    color: [f32; 3],
}

pub fn render(mesh: &Mesh, width: u32, height: u32, rotation: f32) -> RgbaImage {
    let mut image = ImageBuffer::from_pixel(width, height, Rgba([21, 24, 31, 255]));
    if width == 0 || height == 0 {
        return image;
    }
    let mut depth = vec![f32::INFINITY; width as usize * height as usize];
    let light = Vec3::new(-0.45, 0.75, 0.55).normalized();
    let (center, model_scale) = mesh_bounds(mesh);
    let scale = width.min(height) as f32 * 0.38;

    let projected: Vec<_> = mesh
        .vertices
        .iter()
        .map(|vertex| {
            let position = rotate_y((vertex.position - center) * model_scale, rotation);
            let normal = rotate_y(vertex.normal, rotation).normalized();
            let camera_z = position.z + 4.2;
            let perspective = 4.2 / camera_z;
            let diffuse = normal.dot(light).max(0.0);
            let brightness = 0.42 + 0.58 * diffuse;
            ScreenVertex {
                x: width as f32 * 0.5 + position.x * scale * perspective,
                y: height as f32 * 0.58 - position.y * scale * perspective,
                depth: camera_z,
                color: vertex.color.map(|channel| channel as f32 * brightness),
            }
        })
        .collect();

    for triangle in mesh.indices.chunks_exact(3) {
        rasterize_triangle(
            &mut image,
            &mut depth,
            projected[triangle[0] as usize],
            projected[triangle[1] as usize],
            projected[triangle[2] as usize],
        );
    }

    image
}

fn mesh_bounds(mesh: &Mesh) -> (Vec3, f32) {
    let Some(first) = mesh.vertices.first() else {
        return (Vec3::default(), 1.0);
    };
    let mut minimum = first.position;
    let mut maximum = first.position;
    for vertex in &mesh.vertices[1..] {
        minimum.x = minimum.x.min(vertex.position.x);
        minimum.y = minimum.y.min(vertex.position.y);
        minimum.z = minimum.z.min(vertex.position.z);
        maximum.x = maximum.x.max(vertex.position.x);
        maximum.y = maximum.y.max(vertex.position.y);
        maximum.z = maximum.z.max(vertex.position.z);
    }
    let center = (minimum + maximum) * 0.5;
    let extent = (maximum - minimum)
        .x
        .max((maximum - minimum).y)
        .max((maximum - minimum).z);
    (center, 2.0 / extent.max(1.0e-4))
}

fn rotate_y(value: Vec3, angle: f32) -> Vec3 {
    let (sin, cos) = angle.sin_cos();
    Vec3::new(
        value.x * cos + value.z * sin,
        value.y,
        -value.x * sin + value.z * cos,
    )
}

fn rasterize_triangle(
    image: &mut RgbaImage,
    depth_buffer: &mut [f32],
    a: ScreenVertex,
    b: ScreenVertex,
    c: ScreenVertex,
) {
    let area = edge(a, b, c.x, c.y);
    if area.abs() < 1.0e-5 {
        return;
    }
    let min_x = a.x.min(b.x).min(c.x).floor().max(0.0) as u32;
    let max_x = a.x.max(b.x).max(c.x).ceil().min(image.width() as f32 - 1.0) as u32;
    let min_y = a.y.min(b.y).min(c.y).floor().max(0.0) as u32;
    let max_y =
        a.y.max(b.y)
            .max(c.y)
            .ceil()
            .min(image.height() as f32 - 1.0) as u32;

    for y in min_y..=max_y {
        for x in min_x..=max_x {
            let sample_x = x as f32 + 0.5;
            let sample_y = y as f32 + 0.5;
            let wa = edge(b, c, sample_x, sample_y) / area;
            let wb = edge(c, a, sample_x, sample_y) / area;
            let wc = edge(a, b, sample_x, sample_y) / area;
            if wa < 0.0 || wb < 0.0 || wc < 0.0 {
                continue;
            }
            let depth = wa * a.depth + wb * b.depth + wc * c.depth;
            let offset = y as usize * image.width() as usize + x as usize;
            if depth >= depth_buffer[offset] {
                continue;
            }
            depth_buffer[offset] = depth;
            let color: [u8; 3] = std::array::from_fn(|index| {
                (wa * a.color[index] + wb * b.color[index] + wc * c.color[index]).clamp(0.0, 255.0)
                    as u8
            });
            image.put_pixel(x, y, Rgba([color[0], color[1], color[2], 255]));
        }
    }
}

fn edge(a: ScreenVertex, b: ScreenVertex, x: f32, y: f32) -> f32 {
    (x - a.x) * (b.y - a.y) - (y - a.y) * (b.x - a.x)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn apple() -> OrganismRecipe {
        OrganismRecipe::from_yaml(include_str!("../shapes/apple.yaml")).expect("valid apple recipe")
    }

    #[test]
    fn lods_increase_mesh_detail() {
        let recipe = apple();
        let counts: Vec<_> = Lod::ALL
            .into_iter()
            .map(|lod| generate_shape(&recipe, lod).vertices.len())
            .collect();
        assert!(counts.windows(2).all(|pair| pair[0] < pair[1]));
    }

    #[test]
    fn generation_is_deterministic() {
        let recipe = apple().with_seed("same-seed");
        let first = generate_shape(&recipe, Lod::Gameplay);
        let second = generate_shape(&recipe, Lod::Gameplay);
        assert_eq!(first, second);
    }

    #[test]
    fn cache_reuses_seed_and_lod() {
        let mut cache = OrganismMeshCache::default();
        let recipe = apple();
        cache.get(&recipe, Lod::Gameplay);
        cache.get(&recipe, Lod::Gameplay);
        assert_eq!(cache.len(), 1);
        cache.get(&recipe, Lod::Close);
        assert_eq!(cache.len(), 2);
    }

    #[test]
    fn cache_distinguishes_recipe_seeds() {
        let mut cache = OrganismMeshCache::default();
        let first = apple().with_seed("first");
        let second = apple().with_seed("second");
        cache.get(&first, Lod::Gameplay);
        cache.get(&second, Lod::Gameplay);
        assert_eq!(cache.len(), 2);
    }

    #[test]
    fn branch_accepts_grape_clusters() {
        let recipe = OrganismRecipe::from_yaml(
            r#"
name: grape test
seed: vineyard
root:
  kind: sphere
  skin: apple
  children:
    - kind: branch
      children:
        - kind: leaf
        - kind: grape_cluster
"#,
        )
        .expect("semantic recipe");
        let mesh = generate_shape(&recipe, Lod::Gameplay);
        assert!(mesh.vertices.len() > generate_shape(&apple(), Lod::Gameplay).vertices.len());
    }

    #[test]
    fn branch_attachment_creates_dimple() {
        let attached = apple();
        let unattached = OrganismRecipe::from_yaml(
            r#"
name: bare apple
seed: orchard-demo
root:
  kind: sphere
  skin: apple
"#,
        )
        .expect("bare sphere recipe");
        let attached_mesh = generate_shape(&attached, Lod::Gameplay);
        let unattached_mesh = generate_shape(&unattached, Lod::Gameplay);
        assert!(attached_mesh.vertices[0].position.y < unattached_mesh.vertices[0].position.y);
    }

    #[test]
    fn renderer_produces_visible_pixels() {
        let mesh = generate_shape(&apple(), Lod::Distant);
        let image = render(&mesh, 160, 120, 0.3);
        assert_eq!((image.width(), image.height()), (160, 120));
        assert!(image.pixels().any(|pixel| pixel.0 != [21, 24, 31, 255]));
    }
}
