#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

#[cfg(not(target_os = "windows"))]
fn main() {
    eprintln!("nickel-windows-uwp-target supports Windows only");
}

#[cfg(target_os = "windows")]
mod target {
    use std::{
        fs::OpenOptions,
        io::Write,
        path::PathBuf,
        sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        },
        thread,
        time::{Duration, Instant, SystemTime, UNIX_EPOCH},
    };
    use windows::{
        ApplicationModel::Core::*,
        Foundation::TypedEventHandler,
        Storage::ApplicationData,
        UI::Core::*,
        Win32::{
            Foundation::HMODULE,
            Graphics::{
                Direct3D::D3D_DRIVER_TYPE_HARDWARE,
                Direct3D11::{
                    D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_SDK_VERSION, D3D11CreateDevice,
                    ID3D11Device, ID3D11DeviceContext, ID3D11RenderTargetView, ID3D11Texture2D,
                },
                Dxgi::{
                    Common::{
                        DXGI_ALPHA_MODE_IGNORE, DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC,
                    },
                    DXGI_PRESENT, DXGI_SCALING_STRETCH, DXGI_SWAP_CHAIN_DESC1,
                    DXGI_SWAP_EFFECT_FLIP_SEQUENTIAL, DXGI_USAGE_RENDER_TARGET_OUTPUT, IDXGIDevice,
                    IDXGIFactory2, IDXGIOutput, IDXGISwapChain1,
                },
            },
            System::Com::{COINIT_MULTITHREADED, CoInitializeEx, CoUninitialize},
        },
        core::*,
    };

    struct Presenter {
        _device: ID3D11Device,
        context: ID3D11DeviceContext,
        swap_chain: IDXGISwapChain1,
        render_target: ID3D11RenderTargetView,
    }

    impl Presenter {
        fn new(window: &CoreWindow) -> Result<Self> {
            let mut device = None;
            let mut context = None;
            // SAFETY: Windows writes owned device and context interfaces to
            // these outputs. No adapter or feature-level arrays are supplied.
            unsafe {
                D3D11CreateDevice(
                    None,
                    D3D_DRIVER_TYPE_HARDWARE,
                    HMODULE::default(),
                    D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                    None,
                    D3D11_SDK_VERSION,
                    Some(&mut device),
                    None,
                    Some(&mut context),
                )?
            };
            let device = device.expect("D3D11CreateDevice returned no device");
            let context = context.expect("D3D11CreateDevice returned no context");
            let dxgi_device: IDXGIDevice = device.cast()?;
            let adapter = unsafe { dxgi_device.GetAdapter()? };
            let factory: IDXGIFactory2 = unsafe { adapter.GetParent()? };
            let desc = DXGI_SWAP_CHAIN_DESC1 {
                Width: 640,
                Height: 480,
                Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                Stereo: BOOL(0),
                SampleDesc: DXGI_SAMPLE_DESC {
                    Count: 1,
                    Quality: 0,
                },
                BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
                BufferCount: 2,
                Scaling: DXGI_SCALING_STRETCH,
                SwapEffect: DXGI_SWAP_EFFECT_FLIP_SEQUENTIAL,
                AlphaMode: DXGI_ALPHA_MODE_IGNORE,
                Flags: 0,
            };
            // SAFETY: The device and CoreWindow remain alive through the
            // swap-chain lifetime; the descriptor is valid for this call.
            let swap_chain = unsafe {
                factory.CreateSwapChainForCoreWindow(
                    &device,
                    window,
                    &desc,
                    None::<&IDXGIOutput>,
                )?
            };
            let texture: ID3D11Texture2D = unsafe { swap_chain.GetBuffer(0)? };
            let mut render_target = None;
            unsafe { device.CreateRenderTargetView(&texture, None, Some(&mut render_target))? };
            Ok(Self {
                _device: device,
                context,
                swap_chain,
                render_target: render_target.expect("CreateRenderTargetView returned no view"),
            })
        }

        fn present(&self, tick: u64) -> Result<()> {
            let color = if tick.is_multiple_of(2) {
                [0.08, 0.32, 0.72, 1.0]
            } else {
                [0.72, 0.24, 0.08, 1.0]
            };
            // SAFETY: The render target belongs to this live D3D11 device;
            // Present follows the clear before the next frame is submitted.
            unsafe {
                self.context
                    .ClearRenderTargetView(&self.render_target, &color);
                self.swap_chain.Present(1, DXGI_PRESENT(0)).ok()
            }
        }
    }

    #[implement(IFrameworkViewSource)]
    struct ProbeSource;

    impl IFrameworkViewSource_Impl for ProbeSource_Impl {
        fn CreateView(&self) -> Result<IFrameworkView> {
            record("create-view");
            Ok(ProbeView.into())
        }
    }

    #[implement(IFrameworkView)]
    struct ProbeView;

    impl IFrameworkView_Impl for ProbeView_Impl {
        fn Initialize(&self, view: Ref<CoreApplicationView>) -> Result<()> {
            record("initialize");
            view.ok()?.Activated(&TypedEventHandler::new(|_, _| {
                record("activated");
                Ok(())
            }))?;
            Ok(())
        }

        fn SetWindow(&self, window: Ref<CoreWindow>) -> Result<()> {
            record("set-window");
            record(&format!("set-window-visible={}", window.ok()?.Visible()?));
            window.ok()?.VisibilityChanged(&TypedEventHandler::<
                CoreWindow,
                VisibilityChangedEventArgs,
            >::new(|_, args| {
                record(&format!("visibility-changed={}", args.ok()?.Visible()?));
                Ok(())
            }))?;
            Ok(())
        }

        fn Load(&self, _: &HSTRING) -> Result<()> {
            record("load");
            Ok(())
        }

        fn Run(&self) -> Result<()> {
            let window = CoreWindow::GetForCurrentThread()?;
            let closed = Arc::new(AtomicBool::new(false));
            let closed_for_handler = Arc::clone(&closed);
            window.Closed(&TypedEventHandler::<CoreWindow, CoreWindowEventArgs>::new(
                move |_, _| {
                    record("closed");
                    closed_for_handler.store(true, Ordering::Relaxed);
                    Ok(())
                },
            ))?;
            record(&format!("before-activate-visible={}", window.Visible()?));
            window.Activate()?;
            record(&format!("after-activate-visible={}", window.Visible()?));
            record("run");
            let presenter = match Presenter::new(&window) {
                Ok(presenter) => {
                    record("presenter-created");
                    Some(presenter)
                }
                Err(error) => {
                    record(&format!("presenter-create-failed={error}"));
                    None
                }
            };
            let dispatcher = window.Dispatcher()?;
            let mut last_heartbeat = Instant::now();
            let mut tick = 0_u64;
            while !closed.load(Ordering::Relaxed) {
                dispatcher.ProcessEvents(CoreProcessEventsOption::ProcessAllIfPresent)?;
                if last_heartbeat.elapsed() >= Duration::from_secs(1) {
                    let visible = window.Visible()?;
                    record(&format!("event-loop-heartbeat visible={visible}"));
                    if visible && let Some(presenter) = &presenter {
                        match presenter.present(tick) {
                            Ok(()) => record(&format!("present-ok frame={tick}")),
                            Err(error) => {
                                record(&format!("present-failed frame={tick} error={error}"))
                            }
                        }
                    }
                    tick += 1;
                    last_heartbeat = Instant::now();
                }
                thread::sleep(Duration::from_millis(50));
            }
            Ok(())
        }

        fn Uninitialize(&self) -> Result<()> {
            record("uninitialize");
            Ok(())
        }
    }

    fn record(stage: &str) {
        let Ok(data) = ApplicationData::Current() else {
            return;
        };
        let Ok(folder) = data.LocalFolder() else {
            return;
        };
        let Ok(path) = folder.Path() else {
            return;
        };
        let path = PathBuf::from(path.to_string()).join("activation.log");
        let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) else {
            return;
        };
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |time| time.as_secs());
        let _ = writeln!(
            file,
            "unix_seconds={timestamp} pid={} stage={stage}",
            std::process::id()
        );
    }

    pub fn run() -> Result<()> {
        // SAFETY: This process uses a single COM apartment for its lifetime.
        unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.ok()?;
        record("main");
        let source: IFrameworkViewSource = ProbeSource.into();
        let result = CoreApplication::Run(&source);
        record("core-application-returned");
        // SAFETY: Balances the successful CoInitializeEx call above.
        unsafe { CoUninitialize() };
        result
    }
}

#[cfg(target_os = "windows")]
fn main() -> windows::core::Result<()> {
    target::run()
}
