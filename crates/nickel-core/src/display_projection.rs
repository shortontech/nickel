#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProjectionMode {
    InternalOnly,
    Duplicate,
    Extend,
    ExternalOnly,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProjectionOutput {
    pub name: String,
    pub internal: bool,
    pub width: i32,
    pub height: i32,
    pub scale_120: u32,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProjectionPlacement {
    pub name: String,
    pub x: i32,
    pub y: i32,
    pub enabled: bool,
    pub scale_120: u32,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProjectionPlan {
    pub mode: ProjectionMode,
    pub placements: Vec<ProjectionPlacement>,
}

#[derive(Clone, Debug, Default)]
pub struct ProjectionChooser {
    previous: Option<Vec<ProjectionPlacement>>,
    preview: Option<ProjectionPlan>,
}

impl ProjectionChooser {
    pub fn supported(outputs: &[ProjectionOutput]) -> Vec<ProjectionMode> {
        if outputs.len() < 2 {
            return Vec::new();
        }
        let has_internal = outputs.iter().any(|output| output.internal);
        let has_external = outputs.iter().any(|output| !output.internal);
        let mut modes = vec![ProjectionMode::Duplicate, ProjectionMode::Extend];
        if has_internal {
            modes.insert(0, ProjectionMode::InternalOnly);
        }
        if has_external {
            modes.push(ProjectionMode::ExternalOnly);
        }
        modes
    }

    pub fn plan(mode: ProjectionMode, outputs: &[ProjectionOutput]) -> Option<ProjectionPlan> {
        if !Self::supported(outputs).contains(&mode) {
            return None;
        }
        let mut x = 0;
        let placements = outputs
            .iter()
            .map(|output| {
                let enabled = match mode {
                    ProjectionMode::InternalOnly => output.internal,
                    ProjectionMode::ExternalOnly => !output.internal,
                    _ => true,
                };
                let placement = ProjectionPlacement {
                    name: output.name.clone(),
                    x: if mode == ProjectionMode::Extend { x } else { 0 },
                    y: 0,
                    enabled,
                    scale_120: output.scale_120,
                };
                if enabled && mode == ProjectionMode::Extend {
                    x += output.width.max(1);
                }
                placement
            })
            .collect();
        Some(ProjectionPlan { mode, placements })
    }

    pub fn preview(&mut self, current: Vec<ProjectionPlacement>, plan: ProjectionPlan) {
        self.previous = Some(current);
        self.preview = Some(plan);
    }
    pub fn confirm(&mut self) {
        self.previous = None;
        self.preview = None;
    }
    pub fn rollback(&mut self) -> Option<Vec<ProjectionPlacement>> {
        self.preview = None;
        self.previous.take()
    }
    pub fn pending(&self) -> Option<&ProjectionPlan> {
        self.preview.as_ref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn outputs() -> Vec<ProjectionOutput> {
        vec![
            ProjectionOutput {
                name: "eDP-1".into(),
                internal: true,
                width: 1920,
                height: 1080,
                scale_120: 120,
            },
            ProjectionOutput {
                name: "DP-1".into(),
                internal: false,
                width: 2560,
                height: 1440,
                scale_120: 180,
            },
        ]
    }
    #[test]
    fn modes_never_disable_every_output() {
        for mode in ProjectionChooser::supported(&outputs()) {
            assert!(
                ProjectionChooser::plan(mode, &outputs())
                    .unwrap()
                    .placements
                    .iter()
                    .any(|entry| entry.enabled)
            );
        }
    }
    #[test]
    fn preview_rolls_back_exact_previous_layout() {
        let old = vec![ProjectionPlacement {
            name: "eDP-1".into(),
            x: -1920,
            y: 48,
            enabled: true,
            scale_120: 144,
        }];
        let plan = ProjectionChooser::plan(ProjectionMode::Extend, &outputs()).unwrap();
        let mut chooser = ProjectionChooser::default();
        chooser.preview(old.clone(), plan);
        assert_eq!(chooser.rollback(), Some(old));
        assert!(chooser.pending().is_none());
    }
}
