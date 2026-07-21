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

#[cfg(test)]
mod tests {
    use super::{Geometry, PANEL_HEIGHT, centered_in, panel, work_area};

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
                y: 720 - PANEL_HEIGHT,
                width: 1280,
                height: PANEL_HEIGHT,
            }
        );
        assert_eq!(work_area(output).height, 720 - PANEL_HEIGHT);
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
}
