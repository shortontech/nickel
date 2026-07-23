use smithay::utils::{Logical, Point, Size};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConnectedOutput {
    pub name: String,
    pub size: Size<i32, Logical>,
    pub location: Point<i32, Logical>,
    priority: u8,
}

#[derive(Default)]
pub struct OutputLayout {
    outputs: Vec<ConnectedOutput>,
}

impl OutputLayout {
    pub fn connect(
        &mut self,
        name: String,
        size: Size<i32, Logical>,
        priority: u8,
    ) -> Vec<ConnectedOutput> {
        if let Some(output) = self.outputs.iter_mut().find(|output| output.name == name) {
            output.size = size;
            output.priority = priority;
        } else {
            self.outputs.push(ConnectedOutput {
                name,
                size,
                location: (0, 0).into(),
                priority,
            });
        }
        self.reflow()
    }

    pub fn disconnect(&mut self, name: &str) -> Vec<ConnectedOutput> {
        self.outputs.retain(|output| output.name != name);
        self.reflow()
    }

    fn reflow(&mut self) -> Vec<ConnectedOutput> {
        self.outputs
            .sort_by(|left, right| (left.priority, &left.name).cmp(&(right.priority, &right.name)));
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
    fn outputs_are_laid_out_by_stable_priority_and_name() {
        let mut layout = OutputLayout::default();
        layout.connect("DP-1".into(), (1920, 1080).into(), 1);
        let outputs = layout.connect("DVI-I-1".into(), (1920, 1080).into(), 0);
        assert_eq!(outputs[0].name, "DVI-I-1");
        assert_eq!(outputs[0].location, (0, 0).into());
        assert_eq!(outputs[1].name, "DP-1");
        assert_eq!(outputs[1].location, (1920, 0).into());
    }

    #[test]
    fn disconnect_compacts_remaining_outputs() {
        let mut layout = OutputLayout::default();
        layout.connect("DP-1".into(), (1920, 1080).into(), 1);
        layout.connect("HDMI-A-1".into(), (2560, 1440).into(), 1);
        layout.connect("DP-2".into(), (1024, 768).into(), 1);

        let outputs = layout.disconnect("HDMI-A-1");
        assert_eq!(outputs[1].name, "DP-2");
        assert_eq!(outputs[1].location, (1920, 0).into());
    }
}
