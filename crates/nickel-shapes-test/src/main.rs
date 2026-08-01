use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use nickel_shapes_test::{Lod, OrganismMeshCache, OrganismRecipe, creature_state, render};
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::{ElementState, KeyEvent, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{Window, WindowAttributes, WindowId};

mod gpu;

use gpu::GpuPreview;

const DEFAULT_WIDTH: u32 = 800;
const DEFAULT_HEIGHT: u32 = 700;

#[derive(Debug)]
struct Options {
    png: Option<PathBuf>,
    save_state: Option<PathBuf>,
    shape: Option<PathBuf>,
    width: u32,
    height: u32,
    lod: Lod,
    seed: Option<String>,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            png: None,
            save_state: None,
            shape: None,
            width: DEFAULT_WIDTH,
            height: DEFAULT_HEIGHT,
            lod: Lod::Gameplay,
            seed: None,
        }
    }
}

fn main() -> Result<(), String> {
    let options = parse_options(std::env::args().skip(1))?;
    let recipe = load_recipe(&options)?;
    if let Some(path) = &options.save_state {
        let state = creature_state(&recipe)
            .ok_or_else(|| "--save-state requires a creature root".to_owned())?;
        let yaml = yaml_serde::to_string(&state)
            .map_err(|error| format!("failed to serialize creature state: {error}"))?;
        std::fs::write(path, yaml)
            .map_err(|error| format!("failed to write {}: {error}", path.display()))?;
        println!("wrote creature state to {}", path.display());
    }
    if let Some(path) = options.png {
        let mut cache = OrganismMeshCache::default();
        let image = render(
            cache.get(&recipe, options.lod),
            options.width,
            options.height,
            0.35,
        );
        image
            .save(&path)
            .map_err(|error| format!("failed to write {}: {error}", path.display()))?;
        println!(
            "wrote {}x{} LOD {} {} to {}",
            options.width,
            options.height,
            options.lod.index(),
            recipe.name,
            path.display()
        );
        return Ok(());
    }

    let event_loop = EventLoop::new().map_err(|error| error.to_string())?;
    event_loop.set_control_flow(ControlFlow::Poll);
    let mut app = ShapesApp::new(options, recipe);
    event_loop
        .run_app(&mut app)
        .map_err(|error| error.to_string())
}

fn parse_options(arguments: impl Iterator<Item = String>) -> Result<Options, String> {
    let mut options = Options::default();
    let mut arguments = arguments.peekable();
    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "--png" => options.png = Some(PathBuf::from(next_value(&mut arguments, "--png")?)),
            "--shape" => {
                options.shape = Some(PathBuf::from(next_value(&mut arguments, "--shape")?))
            }
            "--save-state" => {
                options.save_state =
                    Some(PathBuf::from(next_value(&mut arguments, "--save-state")?))
            }
            "--width" => options.width = parse_positive(&next_value(&mut arguments, "--width")?)?,
            "--height" => {
                options.height = parse_positive(&next_value(&mut arguments, "--height")?)?
            }
            "--lod" => {
                let value = next_value(&mut arguments, "--lod")?
                    .parse::<u8>()
                    .map_err(|_| "--lod must be 0, 1, 2, or 3".to_owned())?;
                options.lod = Lod::from_index(value)
                    .ok_or_else(|| "--lod must be 0, 1, 2, or 3".to_owned())?;
            }
            "--seed" => {
                options.seed = Some(next_value(&mut arguments, "--seed")?);
            }
            "--help" | "-h" => {
                println!(
                    "nickel-shapes-test [--shape YAML] [--png PATH] [--save-state YAML] [--width PIXELS] \
                     [--height PIXELS] [--lod 0|1|2|3] [--seed TEXT]\n\n\
                     Without --png, opens an interactive window. Press 1-4 to select LOD, \
                     S to save nickel-apple.png, or Escape to exit."
                );
                std::process::exit(0);
            }
            _ => return Err(format!("unknown argument: {argument}; use --help")),
        }
    }
    Ok(options)
}

fn load_recipe(options: &Options) -> Result<OrganismRecipe, String> {
    let (source, directory) = if let Some(path) = &options.shape {
        (
            std::fs::read_to_string(path)
                .map_err(|error| format!("failed to read {}: {error}", path.display()))?,
            path.parent()
                .unwrap_or_else(|| std::path::Path::new("."))
                .to_path_buf(),
        )
    } else {
        (
            include_str!("../shapes/apple.yaml").to_owned(),
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("shapes"),
        )
    };
    let mut recipe = OrganismRecipe::from_yaml(&source)?;
    recipe.resolve_components(&directory)?;
    if let Some(seed) = &options.seed {
        recipe = recipe.with_seed(seed);
    }
    Ok(recipe)
}

fn next_value(
    arguments: &mut impl Iterator<Item = String>,
    option: &str,
) -> Result<String, String> {
    arguments
        .next()
        .ok_or_else(|| format!("{option} requires a value"))
}

fn parse_positive(value: &str) -> Result<u32, String> {
    value
        .parse::<u32>()
        .ok()
        .filter(|value| *value > 0)
        .ok_or_else(|| format!("expected a positive pixel count, got {value}"))
}

struct ShapesApp {
    options: Options,
    recipe: OrganismRecipe,
    cache: OrganismMeshCache,
    window: Option<Arc<Window>>,
    preview: Option<GpuPreview>,
    started: Instant,
    last_angle: f32,
}

impl ShapesApp {
    fn new(options: Options, recipe: OrganismRecipe) -> Self {
        Self {
            recipe,
            options,
            cache: OrganismMeshCache::default(),
            window: None,
            preview: None,
            started: Instant::now(),
            last_angle: 0.0,
        }
    }

    fn draw(&mut self) {
        let Some(preview) = &mut self.preview else {
            return;
        };
        let angle = self.started.elapsed().as_secs_f32() * 0.32;
        preview.render(angle);
        self.last_angle = angle;
    }

    fn set_lod(&mut self, lod: Lod) {
        self.options.lod = lod;
        if let Some(preview) = &mut self.preview {
            preview.set_mesh(self.cache.get(&self.recipe, lod));
        }
    }

    fn handle_key(&mut self, event_loop: &ActiveEventLoop, event: KeyEvent) {
        if event.state != ElementState::Pressed || event.repeat {
            return;
        }
        match event.physical_key {
            PhysicalKey::Code(KeyCode::Escape) => event_loop.exit(),
            PhysicalKey::Code(KeyCode::Digit1) => self.set_lod(Lod::Distant),
            PhysicalKey::Code(KeyCode::Digit2) => self.set_lod(Lod::Gameplay),
            PhysicalKey::Code(KeyCode::Digit3) => self.set_lod(Lod::Close),
            PhysicalKey::Code(KeyCode::Digit4) => self.set_lod(Lod::Inspection),
            PhysicalKey::Code(KeyCode::KeyS) => {
                if let Some(window) = &self.window {
                    let size = window.inner_size();
                    let image = render(
                        self.cache.get(&self.recipe, self.options.lod),
                        size.width,
                        size.height,
                        self.last_angle,
                    );
                    match image.save("nickel-apple.png") {
                        Ok(()) => println!("wrote nickel-apple.png"),
                        Err(error) => eprintln!("failed to write nickel-apple.png: {error}"),
                    }
                }
            }
            _ => {}
        }
    }
}

impl ApplicationHandler for ShapesApp {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let attributes = WindowAttributes::default()
            .with_title(format!(
                "Nickel Shapes Test — {} — 1-4 LOD, S save, Esc exit",
                self.recipe.name
            ))
            .with_inner_size(LogicalSize::new(self.options.width, self.options.height));
        let window = match event_loop.create_window(attributes) {
            Ok(window) => Arc::new(window),
            Err(error) => {
                eprintln!("failed to create window: {error}");
                event_loop.exit();
                return;
            }
        };
        let mesh = self.cache.get(&self.recipe, self.options.lod).clone();
        let preview = match pollster::block_on(GpuPreview::new(window.clone(), &mesh)) {
            Ok(preview) => preview,
            Err(error) => {
                eprintln!("failed to create shader preview: {error}");
                event_loop.exit();
                return;
            }
        };
        self.window = Some(window);
        self.preview = Some(preview);
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _window_id: WindowId,
        event: WindowEvent,
    ) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::KeyboardInput { event, .. } => self.handle_key(event_loop, event),
            WindowEvent::RedrawRequested => self.draw(),
            WindowEvent::Resized(size) => {
                if let Some(preview) = &mut self.preview {
                    preview.resize(size.width, size.height);
                }
                if let Some(window) = &self.window {
                    window.request_redraw();
                }
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_png_options() {
        let arguments = [
            "--shape",
            "pear.yaml",
            "--save-state",
            "creature-state.yaml",
            "--png",
            "fruit.png",
            "--width",
            "320",
            "--height",
            "240",
            "--lod",
            "3",
            "--seed",
            "orchard-row-72",
        ]
        .into_iter()
        .map(str::to_owned);
        let options = parse_options(arguments).expect("valid options");
        assert_eq!(options.shape, Some(PathBuf::from("pear.yaml")));
        assert_eq!(
            options.save_state,
            Some(PathBuf::from("creature-state.yaml"))
        );
        assert_eq!(options.png, Some(PathBuf::from("fruit.png")));
        assert_eq!(options.width, 320);
        assert_eq!(options.height, 240);
        assert_eq!(options.lod, Lod::Inspection);
        assert_eq!(options.seed.as_deref(), Some("orchard-row-72"));
    }
}
