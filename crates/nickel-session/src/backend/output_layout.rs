use smithay::utils::{Logical, Point, Size};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConnectedOutput {
    pub name: String,
    pub size: Size<i32, Logical>,
    pub location: Point<i32, Logical>,
}

#[derive(Default)]
pub struct OutputLayout {
    outputs: Vec<ConnectedOutput>,
}

impl OutputLayout {
    pub fn connect(&mut self, name: String, size: Size<i32, Logical>) -> Point<i32, Logical> {
        if let Some(output) = self.outputs.iter_mut().find(|output| output.name == name) {
            output.size = size;
            return output.location;
        }

        let location = (self.outputs.iter().map(|output| output.size.w).sum(), 0).into();
        self.outputs.push(ConnectedOutput {
            name,
            size,
            location,
        });
        location
    }

    pub fn disconnect(&mut self, name: &str) -> Vec<ConnectedOutput> {
        self.outputs.retain(|output| output.name != name);
        let mut x = 0;
        for output in &mut self.outputs {
            output.location = (x, 0).into();
            x += output.size.w;
        }
        self.outputs.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::OutputLayout;

    #[test]
    fn outputs_are_laid_out_in_stable_connection_order() {
        let mut layout = OutputLayout::default();
        assert_eq!(
            layout.connect("DP-1".into(), (1920, 1080).into()),
            (0, 0).into()
        );
        assert_eq!(
            layout.connect("HDMI-A-1".into(), (2560, 1440).into()),
            (1920, 0).into()
        );
        assert_eq!(
            layout.connect("DP-1".into(), (1280, 720).into()),
            (0, 0).into()
        );
    }

    #[test]
    fn disconnect_compacts_remaining_outputs() {
        let mut layout = OutputLayout::default();
        layout.connect("DP-1".into(), (1920, 1080).into());
        layout.connect("HDMI-A-1".into(), (2560, 1440).into());
        layout.connect("DP-2".into(), (1024, 768).into());

        let outputs = layout.disconnect("HDMI-A-1");
        assert_eq!(outputs[1].name, "DP-2");
        assert_eq!(outputs[1].location, (1920, 0).into());
    }
}
