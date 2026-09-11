//! Exact, bounded ancestry for compositor-owned shell surface incarnations.
//!
//! Platform owners build this projection only from their current protected-
//! filtered surface inventory. A caller-provided id is only a lookup key and
//! can never introduce a parent relationship.

use nickel_remote_control::{
    diagnostics::{MAX_DIAGNOSTIC_SHELL_SURFACES, ShellDiagnosticRole, ShellSurfaceDiagnostic},
    leases::ResourceId,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SurfaceLink {
    pub identity: ResourceId,
    pub parent: Option<ResourceId>,
}

#[derive(Default)]
pub(crate) struct SurfaceAuthority {
    links: Vec<SurfaceLink>,
}

impl SurfaceAuthority {
    pub(crate) fn from_shell_surfaces(surfaces: &[ShellSurfaceDiagnostic]) -> Result<Self, String> {
        let mut links = Vec::with_capacity(surfaces.len());
        for surface in surfaces {
            let identity = ResourceId {
                id: surface.id.clone(),
                generation: surface.generation,
            };
            let parent = if matches!(
                surface.role,
                ShellDiagnosticRole::Launcher
                    | ShellDiagnosticRole::ControlCenter
                    | ShellDiagnosticRole::WindowPreview
            ) {
                let mut panels = surfaces.iter().filter(|candidate| {
                    matches!(candidate.role, ShellDiagnosticRole::Panel)
                        && candidate.output == surface.output
                        && surface.output.is_some()
                });
                let panel = panels.next();
                if panels.next().is_some() {
                    return Err("ambiguous shell surface parent".into());
                }
                panel.map(|panel| ResourceId {
                    id: panel.id.clone(),
                    generation: panel.generation,
                })
            } else {
                None
            };
            links.push(SurfaceLink { identity, parent });
        }
        let mut authority = Self::default();
        authority.reconcile(links)?;
        Ok(authority)
    }

    pub(crate) fn reconcile(&mut self, links: Vec<SurfaceLink>) -> Result<(), String> {
        if links.len() > MAX_DIAGNOSTIC_SHELL_SURFACES
            || links.iter().any(|link| {
                link.identity.id.is_empty()
                    || link.identity.id.len() > 512
                    || link.identity.generation == 0
                    || link.parent.as_ref().is_some_and(|parent| {
                        parent.id.is_empty() || parent.id.len() > 512 || parent.generation == 0
                    })
            })
            || links.iter().enumerate().any(|(index, link)| {
                links[..index]
                    .iter()
                    .any(|prior| prior.identity == link.identity)
            })
        {
            self.links.clear();
            return Err("invalid shell surface ancestry".into());
        }
        for link in &links {
            let mut seen = vec![link.identity.clone()];
            let mut current = link;
            while let Some(parent) = &current.parent {
                if seen.contains(parent) {
                    self.links.clear();
                    return Err("cyclic shell surface ancestry".into());
                }
                seen.push(parent.clone());
                let Some(next) = links.iter().find(|candidate| candidate.identity == *parent)
                else {
                    break;
                };
                current = next;
            }
        }
        self.links = links;
        Ok(())
    }

    /// Return only exact live parent incarnations. Missing parents terminate
    /// the chain, so retirement and id reuse cannot inherit stale authority.
    pub(crate) fn ancestors(&self, identity: &ResourceId) -> Vec<ResourceId> {
        let mut result = Vec::new();
        let Some(mut current) = self.links.iter().find(|link| link.identity == *identity) else {
            return result;
        };
        while let Some(parent) = &current.parent {
            let Some(next) = self
                .links
                .iter()
                .find(|candidate| candidate.identity == *parent)
            else {
                break;
            };
            result.push(parent.clone());
            current = next;
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(name: &str, generation: u64) -> ResourceId {
        ResourceId {
            id: name.into(),
            generation,
        }
    }

    #[test]
    fn exact_live_ancestors_cover_descendants_but_not_unrelated_surfaces() {
        let root = id("panel", 1);
        let child = id("launcher", 2);
        let grandchild = id("menu", 3);
        let unrelated = id("notification", 4);
        let mut authority = SurfaceAuthority::default();
        authority
            .reconcile(vec![
                SurfaceLink {
                    identity: root.clone(),
                    parent: None,
                },
                SurfaceLink {
                    identity: child.clone(),
                    parent: Some(root.clone()),
                },
                SurfaceLink {
                    identity: grandchild.clone(),
                    parent: Some(child.clone()),
                },
                SurfaceLink {
                    identity: unrelated.clone(),
                    parent: None,
                },
            ])
            .unwrap();
        assert_eq!(authority.ancestors(&child), vec![root.clone()]);
        assert_eq!(authority.ancestors(&grandchild), vec![child, root.clone()]);
        assert!(authority.ancestors(&unrelated).is_empty());
        assert!(!authority.ancestors(&unrelated).contains(&root));
    }

    #[test]
    fn parent_retirement_and_generation_reuse_fail_closed() {
        let old_parent = id("panel", 1);
        let child = id("launcher", 2);
        let mut authority = SurfaceAuthority::default();
        authority
            .reconcile(vec![
                SurfaceLink {
                    identity: old_parent.clone(),
                    parent: None,
                },
                SurfaceLink {
                    identity: child.clone(),
                    parent: Some(old_parent.clone()),
                },
            ])
            .unwrap();
        assert_eq!(authority.ancestors(&child), vec![old_parent.clone()]);

        let replacement = id("panel", 3);
        authority
            .reconcile(vec![
                SurfaceLink {
                    identity: replacement.clone(),
                    parent: None,
                },
                SurfaceLink {
                    identity: child.clone(),
                    parent: Some(old_parent),
                },
            ])
            .unwrap();
        assert!(authority.ancestors(&child).is_empty());

        authority
            .reconcile(vec![
                SurfaceLink {
                    identity: replacement.clone(),
                    parent: None,
                },
                SurfaceLink {
                    identity: child.clone(),
                    parent: Some(replacement.clone()),
                },
            ])
            .unwrap();
        assert_eq!(authority.ancestors(&child), vec![replacement]);
    }

    #[test]
    fn malformed_or_cyclic_owner_evidence_clears_authority() {
        let a = id("a", 1);
        let b = id("b", 2);
        let mut authority = SurfaceAuthority::default();
        assert!(
            authority
                .reconcile(vec![
                    SurfaceLink {
                        identity: a.clone(),
                        parent: Some(b.clone()),
                    },
                    SurfaceLink {
                        identity: b,
                        parent: Some(a.clone()),
                    },
                ])
                .is_err()
        );
        assert!(authority.ancestors(&a).is_empty());
    }

    fn surface(
        name: &str,
        generation: u64,
        role: ShellDiagnosticRole,
        output: &str,
    ) -> ShellSurfaceDiagnostic {
        ShellSurfaceDiagnostic {
            id: name.into(),
            generation,
            role,
            geometry: [0, 0, 100, 100],
            output: Some(output.into()),
            scene_generation: 1,
            scale_factor: 1.0,
            redraw_pending: false,
            keyboard_focused: false,
        }
    }

    #[test]
    fn production_roles_link_only_unambiguous_same_output_panel_children() {
        let left_panel = surface("panel-left", 1, ShellDiagnosticRole::Panel, "left");
        let right_panel = surface("panel-right", 2, ShellDiagnosticRole::Panel, "right");
        let launcher = surface("launcher", 3, ShellDiagnosticRole::Launcher, "left");
        let notification = surface("notification", 4, ShellDiagnosticRole::Notification, "left");
        let authority = SurfaceAuthority::from_shell_surfaces(&[
            left_panel.clone(),
            right_panel,
            launcher.clone(),
            notification.clone(),
        ])
        .unwrap();
        assert_eq!(
            authority.ancestors(&ResourceId {
                id: launcher.id,
                generation: launcher.generation,
            }),
            vec![ResourceId {
                id: left_panel.id,
                generation: left_panel.generation,
            }]
        );
        assert!(
            authority
                .ancestors(&ResourceId {
                    id: notification.id,
                    generation: notification.generation,
                })
                .is_empty()
        );
    }
}
