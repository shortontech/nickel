use std::sync::Arc;

use winit::window::Window;

pub struct SharedGraphics {
    instance: wgpu::Instance,
    pub adapter: wgpu::Adapter,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
}

impl SharedGraphics {
    pub async fn new(window: Arc<Window>) -> Result<(Arc<Self>, wgpu::Surface<'static>), String> {
        let instance = wgpu::Instance::default();
        let surface = instance
            .create_surface(window)
            .map_err(|error| format!("failed to create graphics surface: {error}"))?;
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::LowPower,
                compatible_surface: Some(&surface),
                force_fallback_adapter: false,
                ..Default::default()
            })
            .await
            .map_err(|error| format!("failed to find a graphics adapter: {error}"))?;
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("Nickel shared device"),
                memory_hints: wgpu::MemoryHints::MemoryUsage,
                ..Default::default()
            })
            .await
            .map_err(|error| format!("failed to create graphics device: {error}"))?;
        Ok((
            Arc::new(Self {
                instance,
                adapter,
                device,
                queue,
            }),
            surface,
        ))
    }

    pub fn create_surface(&self, window: Arc<Window>) -> Result<wgpu::Surface<'static>, String> {
        self.instance
            .create_surface(window)
            .map_err(|error| format!("failed to create graphics surface: {error}"))
    }
}
