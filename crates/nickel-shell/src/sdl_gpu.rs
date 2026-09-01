use std::{num::NonZeroU32, sync::Arc};

use nickel_ui::backend::PaintCommand;
use nickel_ui::{DamageRegion, PresenterCacheDiagnostics, SdlComponentRenderer};
use raw_window_handle::{
    DisplayHandle, HandleError, HasDisplayHandle, HasWindowHandle, RawDisplayHandle,
    RawWindowHandle, WindowHandle,
};
use sdl3::video::Window;

#[derive(Clone, Copy, Debug)]
struct DisplayHandleSource(RawDisplayHandle);

// SAFETY: SDL owns the native display connection and outlives the shared
// softbuffer context. The copied handle is only consumed by softbuffer.
unsafe impl Send for DisplayHandleSource {}
unsafe impl Sync for DisplayHandleSource {}

impl HasDisplayHandle for DisplayHandleSource {
    fn display_handle(&self) -> Result<DisplayHandle<'_>, HandleError> {
        // SAFETY: SDL keeps this display handle valid for the shell lifetime.
        Ok(unsafe { DisplayHandle::borrow_raw(self.0) })
    }
}

#[derive(Clone, Copy, Debug)]
struct WindowHandleSource(RawWindowHandle);

// SAFETY: presentation remains on the shell thread and the corresponding SDL
// window outlives the presenter containing this copied handle.
unsafe impl Send for WindowHandleSource {}
unsafe impl Sync for WindowHandleSource {}

impl HasWindowHandle for WindowHandleSource {
    fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
        // SAFETY: the owning SDL window remains alive until after this surface.
        Ok(unsafe { WindowHandle::borrow_raw(self.0) })
    }
}

pub struct SharedSdlGraphics {
    context: softbuffer::Context<DisplayHandleSource>,
}

impl SharedSdlGraphics {
    pub fn new(window: &Window) -> Result<Arc<Self>, String> {
        let display = window
            .display_handle()
            .map_err(|error| error.to_string())?
            .as_raw();
        let context = softbuffer::Context::new(DisplayHandleSource(display))
            .map_err(|error| error.to_string())?;
        tracing::info!("shared-memory shell presenter initialized");
        Ok(Arc::new(Self { context }))
    }
}

pub struct SdlGpuPresenter {
    surface: softbuffer::Surface<DisplayHandleSource, WindowHandleSource>,
    renderer: SdlComponentRenderer,
    peak_pixel_bytes: usize,
}

impl SdlGpuPresenter {
    pub fn new(window: &Window, graphics: Arc<SharedSdlGraphics>) -> Result<Self, String> {
        let handle = window
            .window_handle()
            .map_err(|error| error.to_string())?
            .as_raw();
        let surface = softbuffer::Surface::new(&graphics.context, WindowHandleSource(handle))
            .map_err(|error| error.to_string())?;
        let (width, height) = window.size_in_pixels();
        let logical_width = window.size().0.max(1);
        let scale = width as f32 / logical_width as f32;
        let renderer = SdlComponentRenderer::new_pixel_buffer(width, height, scale);
        let peak_pixel_bytes = renderer.pixels().len().saturating_mul(4);
        Ok(Self {
            surface,
            renderer,
            peak_pixel_bytes,
        })
    }

    pub fn present(
        &mut self,
        window: &Window,
        commands: &[PaintCommand],
    ) -> Result<DamageRegion, String> {
        let (width, height) = window.size_in_pixels();
        let width = NonZeroU32::new(width.max(1)).expect("nonzero shell width");
        let height = NonZeroU32::new(height.max(1)).expect("nonzero shell height");
        let logical_width = window.size().0.max(1);
        let scale = width.get() as f32 / logical_width as f32;
        self.renderer.resize(width.get(), height.get(), scale);
        self.peak_pixel_bytes = self
            .peak_pixel_bytes
            .max(self.renderer.pixels().len().saturating_mul(4));
        let damage = self.renderer.render(commands);
        if damage.is_empty() {
            return Ok(damage);
        }
        self.surface
            .resize(width, height)
            .map_err(|error| error.to_string())?;
        let mut buffer = self
            .surface
            .buffer_mut()
            .map_err(|error| error.to_string())?;
        for (target, pixel) in buffer.iter_mut().zip(self.renderer.pixels()) {
            *target = u32::from(pixel.r) << 16 | u32::from(pixel.g) << 8 | u32::from(pixel.b);
        }
        buffer.present().map_err(|error| error.to_string())?;
        Ok(damage)
    }

    pub fn invalidate(&mut self) {
        self.renderer.invalidate();
    }

    pub fn suspend(&mut self) {
        self.renderer.suspend();
    }

    pub fn cache_diagnostics(&self) -> PresenterCacheDiagnostics {
        pixel_buffer_diagnostics(&self.renderer, self.peak_pixel_bytes)
    }
}

fn pixel_buffer_diagnostics(
    renderer: &SdlComponentRenderer,
    peak_pixel_bytes: usize,
) -> PresenterCacheDiagnostics {
    let live_bytes = renderer.pixels().len().saturating_mul(4);
    PresenterCacheDiagnostics {
        image_textures: usize::from(live_bytes > 4),
        live_bytes,
        peak_bytes: peak_pixel_bytes.max(live_bytes),
        ..PresenterCacheDiagnostics::default()
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use nickel_ui::{
        AggregatePresenterCacheDiagnostics, Container, PresenterCacheDiagnostics, Rect,
        SdlComponentRenderer, Text, UiFrame,
    };

    fn p95(mut samples: Vec<Duration>) -> Duration {
        samples.sort_unstable();
        samples[(samples.len() * 95 / 100).min(samples.len() - 1)]
    }

    #[test]
    fn presenter_pixel_buffer_releases_on_suspend_with_measured_reopen_cost() {
        let mut renderer = SdlComponentRenderer::new_pixel_buffer(1920, 1080, 1.0);
        assert_eq!(std::mem::size_of_val(renderer.pixels()), 8_294_400);

        let warm = p95((0..31)
            .map(|_| {
                let started = Instant::now();
                renderer.resize(1920, 1080, 1.0);
                started.elapsed()
            })
            .collect());
        let cold = p95((0..31)
            .map(|_| {
                renderer.suspend();
                assert_eq!(std::mem::size_of_val(renderer.pixels()), 4);
                let started = Instant::now();
                renderer.resize(1920, 1080, 1.0);
                started.elapsed()
            })
            .collect());
        renderer.suspend();

        println!(
            "shell presenter 1080p warm-resize p95={}ns cold-show-allocation p95={}us retained-visible={} retained-hidden={}",
            warm.as_nanos(),
            cold.as_micros(),
            8_294_400,
            std::mem::size_of_val(renderer.pixels())
        );
        assert!(cold > warm);
        assert_eq!(std::mem::size_of_val(renderer.pixels()), 4);
    }

    #[test]
    fn presenter_suspend_and_reopen_preserves_raster_equivalence() {
        let frame = UiFrame::<()>::layout(
            Container::new()
                .width(64.0)
                .height(48.0)
                .background(0x1a2b3c),
            Rect::new(0.0, 0.0, 64.0, 48.0),
        );
        let mut renderer = SdlComponentRenderer::new_pixel_buffer(64, 48, 1.0);
        let _ = renderer.render(frame.commands());
        let before = renderer.pixels().to_vec();

        renderer.suspend();
        renderer.resize(64, 48, 1.0);
        let _ = renderer.render(frame.commands());

        assert_eq!(renderer.pixels(), before);
    }

    #[test]
    fn damaged_warm_renderer_frame_is_allocation_free_and_raster_equivalent() {
        let mut renderer = SdlComponentRenderer::new_pixel_buffer(320, 200, 1.0);
        for generation in 0..33 {
            let frame = UiFrame::<()>::layout(
                Container::new()
                    .width(320.0)
                    .height(200.0)
                    .background(if generation % 2 == 0 {
                        0x111827
                    } else {
                        0x111828
                    })
                    .child(Text::new("Nickel steady frame")),
                Rect::new(0.0, 0.0, 320.0, 200.0),
            );

            let mut reference = SdlComponentRenderer::new_pixel_buffer(320, 200, 1.0);
            reference.render(frame.commands());
            let expected = reference.pixels();

            let Some(before) = crate::allocation_counter::thread_allocation_operations() else {
                // The reusable library test target does not install the native
                // shell binary's allocator. The same test runs with counting
                // enabled in the `nickel` binary test target.
                return;
            };
            let damage = renderer.render(frame.commands());
            let after = crate::allocation_counter::thread_allocation_operations().unwrap();
            assert!(!damage.is_empty());
            assert_eq!(renderer.pixels(), expected);
            if generation > 0 {
                assert_eq!(after - before, 0, "warm damaged frame {generation}");
            }
        }
    }

    #[test]
    fn surface_presenter_close_reopen_has_steady_cache_bounds_and_separate_rss() {
        const SURFACES: usize = 2;
        const WIDTH: u32 = 320;
        const HEIGHT: u32 = 200;
        let cache_growth_budget = include_str!("../../../assets/ui-feedback-budgets.toml")
            .lines()
            .find_map(|line| line.strip_prefix("cache_growth_bytes = "))
            .and_then(|value| value.parse::<usize>().ok())
            .expect("focused cache growth budget remains declared");
        let rss_before = linux_rss_bytes();
        let mut prior_live_bytes = None;

        for generation in 0..4 {
            let mut presenters = (0..SURFACES)
                .map(|_| SdlComponentRenderer::new_pixel_buffer(WIDTH, HEIGHT, 1.0))
                .collect::<Vec<_>>();
            for (index, presenter) in presenters.iter_mut().enumerate() {
                let locale_text =
                    ["Nickel", "نيكل", "ニッケル", "Níquel"][(generation + index) % 4];
                let frame = UiFrame::<()>::layout(
                    Container::new()
                        .width(WIDTH as f32)
                        .height(HEIGHT as f32)
                        .background(if generation % 2 == 0 {
                            0x111827
                        } else {
                            0xf4f7ff
                        })
                        .child(Text::new(locale_text).scale(1.0 + index as f32 * 0.25)),
                    Rect::new(0.0, 0.0, WIDTH as f32, HEIGHT as f32),
                );
                presenter.resize(WIDTH, HEIGHT, 1.0 + generation as f32 * 0.25);
                presenter.render(frame.commands());
            }
            let aggregate = AggregatePresenterCacheDiagnostics::from_presenters(
                presenters.iter().map(|renderer| {
                    super::pixel_buffer_diagnostics(
                        renderer,
                        renderer.pixels().len().saturating_mul(4),
                    )
                }),
            );
            assert_eq!(aggregate.presenters, SURFACES);
            assert_eq!(aggregate.live_entries, SURFACES);
            assert_eq!(
                aggregate.live_bytes,
                SURFACES * WIDTH as usize * HEIGHT as usize * 4
            );
            assert!(aggregate.live_bytes <= cache_growth_budget);
            if let Some(previous) = prior_live_bytes.replace(aggregate.live_bytes) {
                assert_eq!(aggregate.live_bytes, previous);
            }

            for presenter in &mut presenters {
                presenter.suspend();
            }
            let hidden = AggregatePresenterCacheDiagnostics::from_presenters(
                presenters.iter().map(|renderer| {
                    super::pixel_buffer_diagnostics(renderer, WIDTH as usize * HEIGHT as usize * 4)
                }),
            );
            assert_eq!(hidden.presenters, SURFACES);
            assert_eq!(hidden.live_entries, 0);
            assert_eq!(hidden.live_bytes, SURFACES * 4);

            drop(presenters);
            let closed = AggregatePresenterCacheDiagnostics::from_presenters(std::iter::empty::<
                PresenterCacheDiagnostics,
            >());
            assert_eq!(closed, AggregatePresenterCacheDiagnostics::default());
        }

        let rss_after = linux_rss_bytes();
        println!(
            "shell presenter cache-owned steady={} bytes budget={} bytes; allocator-visible rss-before={rss_before:?} rss-after={rss_after:?} delta={:?}",
            prior_live_bytes.unwrap_or_default(),
            cache_growth_budget,
            rss_before
                .zip(rss_after)
                .map(|(before, after)| after as i128 - before as i128),
        );
    }

    #[cfg(target_os = "linux")]
    fn linux_rss_bytes() -> Option<usize> {
        let status = std::fs::read_to_string("/proc/self/status").ok()?;
        super::super::sdl_shell::parse_proc_status_rss(&status)
    }

    #[cfg(not(target_os = "linux"))]
    fn linux_rss_bytes() -> Option<usize> {
        None
    }
}
