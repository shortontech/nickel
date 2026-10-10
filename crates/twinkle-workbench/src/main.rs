//! Standalone component gallery using only Twinkle's production fixture contracts.
use std::{error::Error, sync::Arc};
use twinkle::{Application, Button, Column, Image, Row, Text, ViewContext};
use twinkle_testkit::{ActivationVia, ErasedFixtureSession, FixtureRegistryEntry};

#[derive(Clone, PartialEq)]
enum Message {
    Select(usize),
    Variant(String),
    Activate(ActivationVia),
    Reset,
}

struct Gallery {
    entries: Vec<FixtureRegistryEntry>,
    selected: usize,
    session: Box<dyn ErasedFixtureSession>,
    raster: Arc<image::RgbaImage>,
    revision: u64,
    status: String,
}

impl Gallery {
    fn new() -> Result<Self, Box<dyn Error>> {
        let entries = twinkle_workbench::registry()?;
        let session = entries.first().ok_or("component catalog is empty")?.open();
        let raster = raster(session.as_ref());
        Ok(Self {
            entries,
            selected: 0,
            session,
            raster,
            revision: 1,
            status: String::new(),
        })
    }

    fn refresh(&mut self) {
        self.raster = raster(self.session.as_ref());
        self.revision = self
            .revision
            .checked_add(1)
            .expect("preview generation exhausted");
    }
}

fn raster(session: &dyn ErasedFixtureSession) -> Arc<image::RgbaImage> {
    let raster = session.render(1.0);
    Arc::new(
        image::RgbaImage::from_raw(raster.width, raster.height, raster.rgba)
            .expect("fixture raster dimensions are consistent"),
    )
}

impl Application for Gallery {
    type Message = Message;
    fn title(&self) -> &str {
        "Twinkle component workbench"
    }
    fn initial_size(&self) -> (u32, u32) {
        (1280, 900)
    }
    fn update(&mut self, message: Message) {
        self.status.clear();
        match message {
            Message::Select(index) => {
                if let Some(entry) = self.entries.get(index) {
                    self.selected = index;
                    self.session = entry.open();
                }
            }
            Message::Variant(id) => match self.entries[self.selected].open_variant(&id) {
                Ok(session) => self.session = session,
                Err(error) => self.status = error.to_string(),
            },
            Message::Activate(via) => {
                if let Err(error) = self.session.activate(via) {
                    self.status = error.to_string();
                }
            }
            Message::Reset => self.session.reset(),
        }
        self.refresh();
    }
    fn view(&self, _context: ViewContext) -> impl twinkle::View<Message> {
        let catalog =
            Column::new()
                .width(260.0)
                .gap(6.0)
                .children(self.entries.iter().enumerate().map(|(index, entry)| {
                    Button::new(Message::Select(index), entry.metadata.title).height(36.0)
                }));
        let variants = Row::new().gap(6.0).children(
            self.session
                .metadata()
                .variants
                .iter()
                .map(|variant| Button::new(Message::Variant(variant.id.into()), variant.title)),
        );
        let actions = Row::new()
            .gap(6.0)
            .child(Button::new(Message::Reset, "Reset"))
            .child(Button::new(
                Message::Activate(ActivationVia::Semantic),
                "Semantic activation",
            ))
            .child(Button::new(
                Message::Activate(ActivationVia::Controller),
                "Controller activation",
            ))
            .child(Button::new(
                Message::Activate(ActivationVia::Keyboard),
                "Keyboard activation",
            ));
        let inspection = self.session.inspect();
        Row::new().gap(16.0).child(catalog).child(
            Column::new()
                .gap(8.0)
                .child(Text::new(self.session.metadata().title))
                .child(variants)
                .child(actions)
                .child(Text::new(format!(
                    "{} semantic nodes; frame {}",
                    self.session.semantic_nodes().len(),
                    inspection.frame_generation
                )))
                .child(Text::new(&self.status))
                .child(
                    Image::new(1, self.raster.clone())
                        .generation(self.revision)
                        .width(900.0)
                        .height(700.0)
                        .fit(twinkle::ImageFit::Contain),
                ),
        )
    }
}

fn main() -> Result<(), Box<dyn Error>> {
    let arguments = std::env::args().skip(1).collect::<Vec<_>>();
    match arguments.as_slice() {
        [] => twinkle::run(Gallery::new()?),
        [command] if command == "native" => twinkle::run(Gallery::new()?),
        [command] if command == "list" => {
            for entry in twinkle_workbench::registry()? {
                println!("{}\t{}", entry.metadata.id, entry.metadata.title);
            }
            Ok(())
        }
        [command] if command == "validate" => {
            for entry in twinkle_workbench::registry()? {
                for variant in entry.metadata.variants {
                    let session = entry.open_variant(variant.id)?;
                    if session.render(1.0).rgba != session.render(1.0).rgba {
                        return Err(format!(
                            "{}:{} renders nondeterministically",
                            entry.metadata.id, variant.id
                        )
                        .into());
                    }
                    if !session.inspect().diagnostics.is_empty() {
                        return Err(format!(
                            "{}:{} has layout diagnostics",
                            entry.metadata.id, variant.id
                        )
                        .into());
                    }
                }
            }
            println!("Twinkle component fixtures validated");
            Ok(())
        }
        [command, fixture, variant, output] if command == "render" => {
            let entry = twinkle_workbench::registry()?
                .into_iter()
                .find(|entry| entry.metadata.id == fixture)
                .ok_or("unknown component fixture")?;
            raster(entry.open_variant(variant)?.as_ref()).save(output)?;
            Ok(())
        }
        _ => Err(
            "usage: twinkle-workbench [native|list|validate|render FIXTURE VARIANT OUTPUT.png]"
                .into(),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use twinkle::{ActionKind, SemanticAction, UiHost};

    #[test]
    fn gallery_selection_and_reset_use_production_semantic_dispatch() {
        let app = Gallery::new().unwrap();
        let expected = app.entries[1].metadata.id;
        let mut host = UiHost::new(app, 1280, 900);
        let target = host.semantic_targets_for_message(&Message::Select(1))[0]
            .id
            .clone();
        let outcome = host.perform_controller_semantic_action(
            target,
            SemanticAction::Invoke(ActionKind::Activate),
        );
        assert!(outcome.semantic_failures.is_empty());
        assert_eq!(host.application().session.metadata().id, expected);
        let revision = host.application().revision;
        let target = host.semantic_targets_for_message(&Message::Reset)[0]
            .id
            .clone();
        let outcome = host.perform_controller_semantic_action(
            target,
            SemanticAction::Invoke(ActionKind::Activate),
        );
        assert!(outcome.semantic_failures.is_empty());
        assert_eq!(host.application().session.metadata().id, expected);
        assert!(host.application().revision > revision);
    }
}
