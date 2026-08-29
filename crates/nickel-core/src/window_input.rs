//! Production policy for resolving and reducing ordinary-window pointer presses.

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PointerPosition {
    pub x: f64,
    pub y: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WindowGeometry {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl WindowGeometry {
    pub fn semantic_target(self) -> Option<PointerPosition> {
        (self.width > 0.0 && self.height > 0.0).then_some(PointerPosition {
            x: self.x + self.width / 2.0,
            y: self.y + self.height / 2.0,
        })
    }

    fn contains(self, position: PointerPosition) -> bool {
        position.x >= self.x
            && position.x < self.x + self.width
            && position.y >= self.y
            && position.y < self.y + self.height
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct WindowSurface<Id> {
    pub id: Id,
    pub geometry: WindowGeometry,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum WindowPointerEffect<Id> {
    ActivateWindow(Id),
}

pub fn resolve_semantic_target<Id: PartialEq>(
    surfaces: &[WindowSurface<Id>],
    id: &Id,
) -> Option<PointerPosition> {
    surfaces
        .iter()
        .find(|surface| &surface.id == id)
        .and_then(|surface| surface.geometry.semantic_target())
}

/// Surfaces use compositor stacking order from bottom to top.
pub fn hit_test<Id: Clone>(
    surfaces: &[WindowSurface<Id>],
    position: PointerPosition,
) -> Option<Id> {
    surfaces
        .iter()
        .rev()
        .find(|surface| surface.geometry.contains(position))
        .map(|surface| surface.id.clone())
}

pub fn reduce_pointer_press<Id>(target: Option<Id>) -> Vec<WindowPointerEffect<Id>> {
    target
        .map(WindowPointerEffect::ActivateWindow)
        .into_iter()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{
        PointerPosition, WindowGeometry, WindowPointerEffect, WindowSurface, hit_test,
        reduce_pointer_press, resolve_semantic_target,
    };

    #[test]
    fn semantic_resolution_and_hit_testing_use_production_geometry_and_stacking() {
        let surfaces = vec![
            WindowSurface {
                id: "back",
                geometry: WindowGeometry {
                    x: 10.0,
                    y: 20.0,
                    width: 30.0,
                    height: 40.0,
                },
            },
            WindowSurface {
                id: "front",
                geometry: WindowGeometry {
                    x: 30.0,
                    y: 40.0,
                    width: 50.0,
                    height: 60.0,
                },
            },
        ];

        assert_eq!(
            resolve_semantic_target(&surfaces, &"front"),
            Some(PointerPosition { x: 55.0, y: 70.0 })
        );
        assert_eq!(
            hit_test(&surfaces, PointerPosition { x: 35.0, y: 45.0 }),
            Some("front")
        );
        assert_eq!(
            reduce_pointer_press(hit_test(&surfaces, PointerPosition { x: 15.0, y: 25.0 })),
            vec![WindowPointerEffect::ActivateWindow("back")]
        );
        assert!(
            reduce_pointer_press(hit_test(&surfaces, PointerPosition { x: 100.0, y: 100.0 }))
                .is_empty()
        );
    }
}
