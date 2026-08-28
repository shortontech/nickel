#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConnectedOutput {
    pub name: String,
    pub width: i32,
    pub height: i32,
    pub x: i32,
    pub y: i32,
    priority: u8,
}

#[derive(Debug, Default)]
pub struct OutputLayout {
    outputs: Vec<ConnectedOutput>,
}

impl OutputLayout {
    pub fn connect(
        &mut self,
        name: String,
        width: i32,
        height: i32,
        priority: u8,
    ) -> Vec<ConnectedOutput> {
        if let Some(output) = self.outputs.iter_mut().find(|output| output.name == name) {
            output.width = width;
            output.height = height;
            output.priority = priority;
        } else {
            self.outputs.push(ConnectedOutput {
                name,
                width,
                height,
                x: 0,
                y: 0,
                priority,
            });
        }
        self.reflow();
        self.outputs.clone()
    }

    pub fn disconnect(&mut self, name: &str) -> Vec<ConnectedOutput> {
        self.outputs.retain(|output| output.name != name);
        self.reflow();
        self.outputs.clone()
    }

    pub fn outputs(&self) -> &[ConnectedOutput] {
        &self.outputs
    }

    fn reflow(&mut self) {
        self.outputs
            .sort_by(|left, right| (left.priority, &left.name).cmp(&(right.priority, &right.name)));
        let mut x = 0;
        for output in &mut self.outputs {
            output.x = x;
            output.y = 0;
            x += output.width;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::OutputLayout;

    #[test]
    fn outputs_are_laid_out_by_stable_priority_and_name() {
        let mut layout = OutputLayout::default();
        layout.connect("DP-1".into(), 1920, 1080, 1);
        layout.connect("DVI-I-1".into(), 1280, 720, 0);
        let outputs = layout.connect("DVI-I-2".into(), 800, 600, 0);
        assert_eq!(
            outputs
                .iter()
                .map(|output| {
                    (
                        output.name.as_str(),
                        output.width,
                        output.height,
                        output.x,
                        output.y,
                    )
                })
                .collect::<Vec<_>>(),
            vec![
                ("DVI-I-1", 1280, 720, 0, 0),
                ("DVI-I-2", 800, 600, 1280, 0),
                ("DP-1", 1920, 1080, 2080, 0),
            ]
        );
    }
}
