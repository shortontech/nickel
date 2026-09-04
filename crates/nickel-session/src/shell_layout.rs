pub const PANEL_HEIGHT: i32 = 56;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Geometry {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

pub fn panel(output: Geometry) -> Geometry {
    let height = PANEL_HEIGHT.min(output.height.max(0));
    Geometry {
        x: output.x,
        y: output.y + output.height - height,
        width: output.width.max(0),
        height,
    }
}

pub fn work_area(output: Geometry) -> Geometry {
    let panel = panel(output);
    Geometry {
        x: output.x,
        y: output.y,
        width: output.width.max(0),
        height: (panel.y - output.y).max(0),
    }
}

pub fn output_for_window(window: Geometry, outputs: &[Geometry]) -> Option<Geometry> {
    outputs.iter().copied().max_by_key(|output| {
        let left = window.x.max(output.x);
        let top = window.y.max(output.y);
        let right = (window.x + window.width).min(output.x + output.width);
        let bottom = (window.y + window.height).min(output.y + output.height);
        i64::from((right - left).max(0)) * i64::from((bottom - top).max(0))
    })
}

pub fn centered_in(area: Geometry, requested: (i32, i32)) -> Geometry {
    let width = requested.0.max(1).min(area.width.max(1));
    let height = requested.1.max(1).min(area.height.max(1));
    Geometry {
        x: area.x + (area.width - width).max(0) / 2,
        y: area.y + (area.height - height).max(0) / 2,
        width,
        height,
    }
}

pub fn bottom_left_in(
    area: Geometry,
    requested: (i32, i32),
    left_inset: i32,
    bottom_gap: i32,
) -> Geometry {
    let mut geometry = centered_in(area, requested);
    geometry.x = area.x + left_inset.max(0).min((area.width - geometry.width).max(0));
    geometry.y = area.y + (area.height - geometry.height - bottom_gap.max(0)).max(0);
    geometry
}

pub fn anchored_popover(
    area: Geometry,
    anchor: Geometry,
    requested: (i32, i32),
    preferred: nickel_session_protocol::AnchorSide,
) -> Geometry {
    use nickel_session_protocol::AnchorSide;

    let width = requested.0.max(1).min(area.width.max(1));
    let height = requested.1.max(1).min(area.height.max(1));
    let gap = 4;
    let candidate = |side| match side {
        AnchorSide::Above => Geometry {
            x: anchor.x + anchor.width - width,
            y: anchor.y - height - gap,
            width,
            height,
        },
        AnchorSide::Below => Geometry {
            x: anchor.x + anchor.width - width,
            y: anchor.y + anchor.height + gap,
            width,
            height,
        },
        AnchorSide::Left => Geometry {
            x: anchor.x - width - gap,
            y: anchor.y + (anchor.height - height) / 2,
            width,
            height,
        },
        AnchorSide::Right => Geometry {
            x: anchor.x + anchor.width + gap,
            y: anchor.y + (anchor.height - height) / 2,
            width,
            height,
        },
    };
    let opposite = match preferred {
        AnchorSide::Above => AnchorSide::Below,
        AnchorSide::Below => AnchorSide::Above,
        AnchorSide::Left => AnchorSide::Right,
        AnchorSide::Right => AnchorSide::Left,
    };
    let fits = |side: AnchorSide, geometry: Geometry| match side {
        AnchorSide::Above | AnchorSide::Below => {
            geometry.y >= area.y && geometry.y + geometry.height <= area.y + area.height
        }
        AnchorSide::Left | AnchorSide::Right => {
            geometry.x >= area.x && geometry.x + geometry.width <= area.x + area.width
        }
    };
    let preferred_geometry = candidate(preferred);
    let mut geometry = if fits(preferred, preferred_geometry) {
        preferred_geometry
    } else {
        let flipped = candidate(opposite);
        if fits(opposite, flipped) {
            flipped
        } else {
            preferred_geometry
        }
    };
    geometry.x = geometry
        .x
        .clamp(area.x, area.x + (area.width - width).max(0));
    geometry.y = geometry
        .y
        .clamp(area.y, area.y + (area.height - height).max(0));
    geometry
}

pub fn initial_window(area: Geometry, cascade: i32) -> Geometry {
    let target_width = (area.width * 3 / 4).clamp(640, 1200);
    let target_height = (area.height * 3 / 4).clamp(480, 800);
    let mut geometry = centered_in(area, (target_width, target_height));
    let maximum_x = area.x + (area.width - geometry.width).max(0);
    let maximum_y = area.y + (area.height - geometry.height).max(0);
    geometry.x = (geometry.x + cascade).clamp(area.x, maximum_x);
    geometry.y = (geometry.y + cascade).clamp(area.y, maximum_y);
    geometry
}

pub fn space_location_for_bounds(target: Geometry, surface_geometry: Geometry) -> (i32, i32) {
    (target.x - surface_geometry.x, target.y - surface_geometry.y)
}

#[cfg(test)]
mod tests {
    use super::{
        Geometry, anchored_popover, bottom_left_in, centered_in, initial_window, output_for_window,
        panel, space_location_for_bounds, work_area,
    };
    use nickel_session_protocol::AnchorSide;

    #[test]
    fn anchored_popovers_flip_slide_and_stay_in_negative_origin_work_areas() {
        let area = Geometry {
            x: -1920,
            y: -200,
            width: 1920,
            height: 1000,
        };
        let bottom_right = Geometry {
            x: -40,
            y: 750,
            width: 32,
            height: 56,
        };
        let placed = anchored_popover(area, bottom_right, (420, 600), AnchorSide::Above);
        assert_eq!(
            placed,
            Geometry {
                x: -428,
                y: 146,
                width: 420,
                height: 600
            }
        );

        let top = Geometry {
            x: -1900,
            y: -190,
            width: 40,
            height: 40,
        };
        let flipped = anchored_popover(area, top, (500, 300), AnchorSide::Above);
        assert_eq!(flipped.y, -146);
        assert_eq!(
            flipped.x, -1920,
            "placement slides within the invoking output"
        );
    }

    #[test]
    fn panel_occupies_bottom_edge_and_work_area_stops_above_it() {
        let output = Geometry {
            x: 0,
            y: 0,
            width: 1280,
            height: 720,
        };
        assert_eq!(
            panel(output),
            Geometry {
                x: 0,
                y: 664,
                width: 1280,
                height: 56,
            }
        );
        assert_eq!(work_area(output).height, 664);
    }

    #[test]
    fn launcher_is_bottom_left_and_attached_above_the_panel() {
        let output = Geometry {
            x: 0,
            y: 0,
            width: 1280,
            height: 800,
        };
        assert_eq!(
            bottom_left_in(work_area(output), (920, 680), 18, 8),
            Geometry {
                x: 18,
                y: 56,
                width: 920,
                height: 680,
            }
        );
    }

    #[test]
    fn launcher_is_centered_and_clamped_to_small_outputs() {
        let area = Geometry {
            x: 10,
            y: 20,
            width: 800,
            height: 544,
        };
        assert_eq!(
            centered_in(area, (400, 300)),
            Geometry {
                x: 210,
                y: 142,
                width: 400,
                height: 300,
            }
        );
        assert_eq!(centered_in(area, (1200, 900)).width, 800);
        assert_eq!(centered_in(area, (1200, 900)).height, 544);
    }

    #[test]
    fn undersized_output_gives_all_available_height_to_panel() {
        let output = Geometry {
            x: 0,
            y: 0,
            width: 40,
            height: 20,
        };
        assert_eq!(panel(output).height, 20);
        assert_eq!(work_area(output).height, 0);
    }

    #[test]
    fn initial_windows_are_usefully_sized_and_remain_inside_work_area() {
        let area = Geometry {
            x: 0,
            y: 0,
            width: 1920,
            height: 1024,
        };
        let window = initial_window(area, 224);
        assert_eq!((window.width, window.height), (1200, 768));
        assert!(window.x >= area.x);
        assert!(window.y >= area.y);
        assert!(window.x + window.width <= area.x + area.width);
        assert!(window.y + window.height <= area.y + area.height);
    }

    #[test]
    fn window_uses_output_with_largest_overlap() {
        let left = Geometry {
            x: 0,
            y: 0,
            width: 1920,
            height: 1080,
        };
        let right = Geometry {
            x: 1920,
            y: 0,
            width: 1920,
            height: 1080,
        };
        let window = Geometry {
            x: 1800,
            y: 100,
            width: 800,
            height: 600,
        };

        assert_eq!(output_for_window(window, &[left, right]), Some(right));
    }

    #[test]
    fn shell_bounds_cancel_client_internal_offsets() {
        let target = Geometry {
            x: 1920,
            y: 1024,
            width: 1920,
            height: 56,
        };
        let surface = Geometry {
            x: 456,
            y: 224,
            width: 1920,
            height: 56,
        };
        assert_eq!(space_location_for_bounds(target, surface), (1464, 800));
    }
}
