#![cfg_attr(not(target_os = "windows"), allow(unused))]

#[cfg(target_os = "windows")]
fn main() -> windows::core::Result<()> {
    use windows::{
        Win32::{
            Media::Audio::{
                IAudioSessionControl2, IAudioSessionManager2, IMMDeviceEnumerator,
                ISimpleAudioVolume, MMDeviceEnumerator, eMultimedia, eRender,
            },
            System::Com::{
                CLSCTX_ALL, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoUninitialize,
            },
        },
        core::Interface,
    };

    // SAFETY: This diagnostic owns its COM apartment and releases every interface before
    // uninitializing it at process exit.
    unsafe {
        CoInitializeEx(None, COINIT_MULTITHREADED).ok()?;
        let devices: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
        let endpoint = devices.GetDefaultAudioEndpoint(eRender, eMultimedia)?;
        let manager: IAudioSessionManager2 = endpoint.Activate(CLSCTX_ALL, None)?;
        let sessions = manager.GetSessionEnumerator()?;
        for index in 0..sessions.GetCount()? {
            let session = sessions.GetSession(index)?;
            let control: IAudioSessionControl2 = session.cast()?;
            let volume: ISimpleAudioVolume = session.cast()?;
            println!(
                "pid={} muted={} volume={:.0}%",
                control.GetProcessId()?,
                volume.GetMute()?.as_bool(),
                volume.GetMasterVolume()? * 100.0,
            );
        }
        CoUninitialize();
    }
    Ok(())
}

#[cfg(not(target_os = "windows"))]
fn main() {
    eprintln!("audio session probing is currently implemented only on Windows");
}
