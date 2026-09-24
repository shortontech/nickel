use std::{
    ffi::c_void,
    mem::size_of,
    os::windows::ffi::{OsStrExt, OsStringExt},
    path::Path,
};

use image::RgbaImage;
use nickel_core::theme::{Appearance, ThemeMode, ThemePalette};
use windows::{
    Win32::{
        Graphics::Gdi::{
            BI_RGB, BITMAP, BITMAPINFO, BITMAPINFOHEADER, CreateCompatibleDC, CreateDIBSection,
            DIB_RGB_COLORS, DeleteDC, DeleteObject, GetDC, GetDIBits, GetObjectW, HGDIOBJ,
            ReleaseDC, SelectObject,
        },
        Storage::FileSystem::FILE_FLAGS_AND_ATTRIBUTES,
        System::Com::{
            CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
            CoUninitialize, IPersistFile, STGM_READ,
        },
        UI::{
            Shell::{
                IShellItemImageFactory, IShellLinkW, SHCreateItemFromParsingName,
                SHDefExtractIconW, SHFILEINFOW, SHGFI_ICON, SHGetFileInfoW, SIIGBF_BIGGERSIZEOK,
                SIIGBF_ICONONLY,
            },
            WindowsAndMessaging::{DI_NORMAL, DestroyIcon, DrawIconEx, HICON},
        },
    },
    core::{GUID, Interface, PCWSTR},
};
use winit::{
    platform::windows::{Color, WindowExtWindows},
    window::{Theme, Window},
};

pub fn show_hidden_files() -> bool {
    use windows::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_DWORD, RegGetValueW};
    use windows::core::w;

    let mut value = 0_u32;
    let mut size = std::mem::size_of::<u32>() as u32;
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            w!("Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\Advanced"),
            w!("Hidden"),
            RRF_RT_REG_DWORD,
            None,
            Some((&raw mut value).cast()),
            Some(&raw mut size),
        )
    };
    status.is_ok() && value == 1
}

pub fn appearance() -> Appearance {
    let light = registry_dword(
        "Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize",
        "SystemUsesLightTheme",
    )
    .unwrap_or(0)
        != 0;
    let accent = registry_dword("Software\\Microsoft\\Windows\\DWM", "AccentColor")
        .map(|value| {
            [
                (value & 0xff) as u8,
                ((value >> 8) & 0xff) as u8,
                ((value >> 16) & 0xff) as u8,
            ]
        })
        .unwrap_or(Appearance::default().accent);
    Appearance {
        mode: if light {
            ThemeMode::Light
        } else {
            ThemeMode::Dark
        },
        accent,
        intensity: registry_dword(
            "Software\\Microsoft\\Windows\\DWM",
            "ColorizationColorBalance",
        )
        .unwrap_or(85)
        .min(100) as u8,
    }
}

pub fn apply_window_appearance(window: &Window, appearance: Appearance) {
    let palette = ThemePalette::from_appearance(appearance);
    window.set_theme(Some(match appearance.mode {
        ThemeMode::Light => Theme::Light,
        ThemeMode::Dark => Theme::Dark,
    }));
    window.set_title_background_color(Some(color(palette.panel)));
    window.set_title_text_color(color(palette.text));
    window.set_border_color(Some(color(palette.accent)));
}

/// Publish Nickel's chosen accent to the per-user Windows personalization values.
/// Windows uses BGR for AccentColor and ARGB for DWM colorization; notify open
/// windows after the values are written so their chrome updates immediately.
pub fn publish_system_accent(accent: u32) -> Result<(), String> {
    use windows::Win32::{
        Foundation::{LPARAM, WPARAM},
        UI::WindowsAndMessaging::{HWND_BROADCAST, SendNotifyMessageW, WM_SETTINGCHANGE},
    };
    use windows::core::w;

    const DWM: &str = "Software\\Microsoft\\Windows\\DWM";
    const EXPLORER_ACCENT: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\Accent";
    let (accent_color, colorization_color) = windows_accent_values(accent);
    set_registry_dword(DWM, "AccentColor", accent_color)?;
    set_registry_dword(DWM, "ColorizationColor", colorization_color)?;
    set_registry_dword(EXPLORER_ACCENT, "AccentColorMenu", accent_color)?;
    set_registry_dword(DWM, "ColorPrevalence", 1)?;
    unsafe {
        SendNotifyMessageW(
            HWND_BROADCAST,
            WM_SETTINGCHANGE,
            WPARAM(0),
            LPARAM(w!("ImmersiveColorSet").as_ptr() as isize),
        )
    }
    .map_err(|error| format!("could not notify Windows of the accent change: {error}"))
}

fn windows_accent_values(accent: u32) -> (u32, u32) {
    let red = (accent >> 16) & 0xff;
    let green = (accent >> 8) & 0xff;
    let blue = accent & 0xff;
    (
        0xff00_0000 | (blue << 16) | (green << 8) | red,
        0xc400_0000 | (red << 16) | (green << 8) | blue,
    )
}

fn set_registry_dword(subkey: &str, name: &str, value: u32) -> Result<(), String> {
    use windows::Win32::System::Registry::{HKEY_CURRENT_USER, REG_DWORD, RegSetKeyValueW};

    let subkey = subkey.encode_utf16().chain([0]).collect::<Vec<_>>();
    let name = name.encode_utf16().chain([0]).collect::<Vec<_>>();
    let result = unsafe {
        RegSetKeyValueW(
            HKEY_CURRENT_USER,
            PCWSTR(subkey.as_ptr()),
            PCWSTR(name.as_ptr()),
            REG_DWORD.0,
            Some((&raw const value).cast()),
            size_of::<u32>() as u32,
        )
    };
    if result.is_ok() {
        Ok(())
    } else {
        Err(format!(
            "could not write Windows accent setting: {result:?}"
        ))
    }
}

#[cfg(test)]
mod accent_tests {
    #[test]
    fn windows_accent_values_use_the_expected_channel_order() {
        assert_eq!(
            super::windows_accent_values(0x123456),
            (0xff56_3412, 0xc412_3456)
        );
    }
}

fn color(rgb: u32) -> Color {
    Color::from_rgb(
        ((rgb >> 16) & 0xff) as u8,
        ((rgb >> 8) & 0xff) as u8,
        (rgb & 0xff) as u8,
    )
}

fn registry_dword(subkey: &str, value_name: &str) -> Option<u32> {
    use windows::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_DWORD, RegGetValueW};

    let subkey: Vec<u16> = subkey.encode_utf16().chain([0]).collect();
    let value_name: Vec<u16> = value_name.encode_utf16().chain([0]).collect();
    let mut value = 0_u32;
    let mut size = size_of::<u32>() as u32;
    unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            PCWSTR(subkey.as_ptr()),
            PCWSTR(value_name.as_ptr()),
            RRF_RT_REG_DWORD,
            None,
            Some((&raw mut value).cast()),
            Some(&raw mut size),
        )
    }
    .is_ok()
    .then_some(value)
}

pub fn path_icon(path: &Path) -> Option<RgbaImage> {
    path_icon_at_size(path, 32)
}

pub fn path_icon_at_size(path: &Path, physical_size: u32) -> Option<RgbaImage> {
    let physical_size = physical_size.clamp(16, 512);
    let initialized = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }.is_ok();
    let shortcut = shortcut_icon(path, physical_size);
    let (image, resolver) = if shortcut.is_some() {
        (shortcut, "shortcut")
    } else if let Some(internet_shortcut) = internet_shortcut_icon(path, physical_size) {
        (Some(internet_shortcut), "internet-shortcut")
    } else if path
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("exe"))
    {
        (
            extract_icon(path, 0, physical_size).or_else(|| shell_path_icon(path, physical_size)),
            "executable",
        )
    } else {
        (
            shell_path_icon(path, physical_size)
                .or_else(|| shell_parsing_name_icon(path, physical_size)),
            "shell-path",
        )
    };
    if initialized {
        unsafe { CoUninitialize() };
    }
    if image
        .as_ref()
        .is_some_and(|image| image.pixels().any(|pixel| pixel.0[3] != 0))
    {
        if resolver == "shell-path"
            && path
                .extension()
                .is_some_and(|extension| extension.eq_ignore_ascii_case("lnk"))
        {
            tracing::warn!(
                path = %path.display(),
                "shortcut-specific icon resolution failed; using shell fallback"
            );
        } else {
            tracing::debug!(path = %path.display(), resolver, "resolved platform icon");
        }
    } else {
        tracing::warn!(
            path = %path.display(),
            resolver,
            "platform icon resolution returned no visible pixels"
        );
    }
    image
}

fn internet_shortcut_icon(path: &Path, physical_size: u32) -> Option<RgbaImage> {
    if !path
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("url"))
    {
        return None;
    }
    let contents = std::fs::read_to_string(path).ok()?;
    let mut icon_path = None;
    let mut icon_index = 0;
    for line in contents.lines() {
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        if key.trim().eq_ignore_ascii_case("IconFile") {
            icon_path = Some(std::path::PathBuf::from(value.trim()));
        } else if key.trim().eq_ignore_ascii_case("IconIndex") {
            icon_index = value.trim().parse().unwrap_or(0);
        }
    }
    let icon_path = icon_path?;
    extract_icon(&icon_path, icon_index, physical_size)
        .or_else(|| shell_path_icon(&icon_path, physical_size))
}

fn shortcut_icon(path: &Path, physical_size: u32) -> Option<RgbaImage> {
    if !path
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("lnk"))
    {
        return None;
    }

    const CLSID_SHELL_LINK: GUID = GUID::from_u128(0x00021401_0000_0000_c000_000000000046);
    let shortcut: IShellLinkW =
        unsafe { CoCreateInstance(&CLSID_SHELL_LINK, None, CLSCTX_INPROC_SERVER) }.ok()?;
    let persisted: IPersistFile = shortcut.cast().ok()?;
    let wide = terminated(path);
    unsafe { persisted.Load(PCWSTR(wide.as_ptr()), STGM_READ) }.ok()?;

    let mut location = vec![0_u16; 32_768];
    let mut index = 0;
    unsafe { shortcut.GetIconLocation(&mut location, &raw mut index) }.ok()?;
    if string_length(&location) == 0 {
        unsafe { shortcut.GetPath(&mut location, std::ptr::null_mut(), 0) }.ok()?;
        let length = string_length(&location);
        if length == 0 {
            return None;
        }
        let target = std::path::PathBuf::from(std::ffi::OsString::from_wide(&location[..length]));
        tracing::debug!(
            shortcut = %path.display(),
            target = %target.display(),
            "shortcut has no explicit icon; resolving its target"
        );
        return shell_path_icon(&target, physical_size);
    }
    let length = string_length(&location);
    if length == 0 {
        return None;
    }
    location.truncate(length + 1);
    let explicit_path =
        std::path::PathBuf::from(std::ffi::OsString::from_wide(&location[..length]));

    let Some(image) = extract_icon(&explicit_path, index, physical_size) else {
        tracing::debug!(
            shortcut = %path.display(),
            icon = %explicit_path.display(),
            index,
            "indexed shortcut icon extraction failed; trying shell path"
        );
        return shell_path_icon(&explicit_path, physical_size);
    };
    Some(image)
}

/// Resolves the executable target recorded by a Windows shell shortcut.
pub fn shortcut_target(path: &Path) -> Option<std::path::PathBuf> {
    if !path
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("lnk"))
    {
        return None;
    }
    let initialized = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }.is_ok();
    let target = shortcut_target_in_apartment(path);
    if initialized {
        unsafe { CoUninitialize() };
    }
    target
}

fn shortcut_target_in_apartment(path: &Path) -> Option<std::path::PathBuf> {
    const CLSID_SHELL_LINK: GUID = GUID::from_u128(0x00021401_0000_0000_c000_000000000046);
    let shortcut: IShellLinkW =
        unsafe { CoCreateInstance(&CLSID_SHELL_LINK, None, CLSCTX_INPROC_SERVER) }.ok()?;
    let persisted: IPersistFile = shortcut.cast().ok()?;
    let wide = terminated(path);
    unsafe { persisted.Load(PCWSTR(wide.as_ptr()), STGM_READ) }.ok()?;
    let mut target = vec![0_u16; 32_768];
    unsafe { shortcut.GetPath(&mut target, std::ptr::null_mut(), 0) }.ok()?;
    let length = string_length(&target);
    (length != 0)
        .then(|| std::path::PathBuf::from(std::ffi::OsString::from_wide(&target[..length])))
}

fn extract_icon(path: &Path, index: i32, physical_size: u32) -> Option<RgbaImage> {
    let wide = terminated(path);
    let mut icon = HICON::default();
    // Ask the shell extractor for the requested large-icon dimensions so PE resources with
    // multiple variants select their best source instead of scaling ExtractIconExW's fixed
    // small/large result.
    let result = unsafe {
        SHDefExtractIconW(
            PCWSTR(wide.as_ptr()),
            index,
            0,
            Some(&raw mut icon),
            None,
            physical_size.clamp(16, 512),
        )
    };
    if result.is_err() || icon.0.is_null() {
        return None;
    }
    let image = render_icon(icon, physical_size);
    unsafe {
        let _ = DestroyIcon(icon);
    }
    image
}

fn shell_path_icon(path: &Path, physical_size: u32) -> Option<RgbaImage> {
    let wide = terminated(path);
    let mut info = SHFILEINFOW::default();
    unsafe {
        let result = SHGetFileInfoW(
            PCWSTR(wide.as_ptr()),
            FILE_FLAGS_AND_ATTRIBUTES(0),
            Some(&raw mut info),
            size_of::<SHFILEINFOW>() as u32,
            SHGFI_ICON,
        );
        if result == 0 || info.hIcon.0.is_null() {
            return None;
        }
        let image = render_icon(info.hIcon, physical_size);
        let _ = DestroyIcon(info.hIcon);
        image
    }
}

fn shell_parsing_name_icon(path: &Path, physical_size: u32) -> Option<RgbaImage> {
    let reference = path.as_os_str().to_string_lossy();
    shell_parsing_name_icon_for(&reference, physical_size).or_else(|| {
        reference.contains('!').then_some(())?;
        shell_parsing_name_icon_for(&format!(r"shell:AppsFolder\{reference}"), physical_size)
    })
}

fn shell_parsing_name_icon_for(reference: &str, physical_size: u32) -> Option<RgbaImage> {
    let wide = reference.encode_utf16().chain([0]).collect::<Vec<_>>();
    let factory: IShellItemImageFactory =
        unsafe { SHCreateItemFromParsingName(PCWSTR(wide.as_ptr()), None) }.ok()?;
    let size = physical_size.clamp(16, 512) as i32;
    let bitmap = unsafe {
        factory.GetImage(
            windows::Win32::Foundation::SIZE { cx: size, cy: size },
            SIIGBF_ICONONLY | SIIGBF_BIGGERSIZEOK,
        )
    }
    .ok()?;
    let image = render_bitmap(bitmap);
    unsafe {
        let _ = DeleteObject(HGDIOBJ(bitmap.0));
    }
    image
}

fn render_bitmap(bitmap: windows::Win32::Graphics::Gdi::HBITMAP) -> Option<RgbaImage> {
    let mut object = BITMAP::default();
    if unsafe {
        GetObjectW(
            HGDIOBJ(bitmap.0),
            size_of::<BITMAP>() as i32,
            Some((&raw mut object).cast()),
        )
    } == 0
        || object.bmWidth <= 0
        || object.bmHeight <= 0
    {
        return None;
    }
    let width = object.bmWidth as u32;
    let height = object.bmHeight as u32;
    let mut info = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: width as i32,
            biHeight: -(height as i32),
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut bgra = vec![0_u8; width as usize * height as usize * 4];
    let screen = unsafe { GetDC(None) };
    if screen.0.is_null() {
        return None;
    }
    let rows = unsafe {
        GetDIBits(
            screen,
            bitmap,
            0,
            height,
            Some(bgra.as_mut_ptr().cast()),
            &raw mut info,
            DIB_RGB_COLORS,
        )
    };
    unsafe {
        ReleaseDC(None, screen);
    }
    if rows != height as i32 {
        return None;
    }
    for pixel in bgra.chunks_exact_mut(4) {
        pixel.swap(0, 2);
    }
    RgbaImage::from_raw(width, height, bgra)
}

fn render_icon(icon: HICON, physical_size: u32) -> Option<RgbaImage> {
    let size = physical_size.clamp(16, 512);
    let info = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: size as i32,
            biHeight: -(size as i32),
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut pixels = std::ptr::null_mut::<c_void>();
    unsafe {
        let screen = GetDC(None);
        if screen.0.is_null() {
            return None;
        }
        let memory = CreateCompatibleDC(Some(screen));
        if memory.0.is_null() {
            ReleaseDC(None, screen);
            return None;
        }
        let bitmap = CreateDIBSection(
            Some(screen),
            &raw const info,
            DIB_RGB_COLORS,
            &raw mut pixels,
            None,
            0,
        )
        .ok()?;
        let previous = SelectObject(memory, HGDIOBJ(bitmap.0));
        let drawn = DrawIconEx(
            memory,
            0,
            0,
            icon,
            size as i32,
            size as i32,
            0,
            None,
            DI_NORMAL,
        )
        .is_ok();
        let mut rgba = vec![0_u8; (size * size * 4) as usize];
        if drawn && !pixels.is_null() {
            let bgra = std::slice::from_raw_parts(pixels.cast::<u8>(), rgba.len());
            for (source, target) in bgra.chunks_exact(4).zip(rgba.chunks_exact_mut(4)) {
                target.copy_from_slice(&[source[2], source[1], source[0], source[3]]);
            }
        }
        SelectObject(memory, previous);
        let _ = DeleteObject(HGDIOBJ(bitmap.0));
        let _ = DeleteDC(memory);
        ReleaseDC(None, screen);
        drawn
            .then(|| RgbaImage::from_raw(size, size, rgba))
            .flatten()
    }
}

fn terminated(path: &Path) -> Vec<u16> {
    path.as_os_str().encode_wide().chain(Some(0)).collect()
}

fn string_length(value: &[u16]) -> usize {
    value
        .iter()
        .position(|character| *character == 0)
        .unwrap_or(value.len())
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{path_icon_at_size, shortcut_target, string_length, terminated};

    #[test]
    fn utf16_helpers_terminate_and_measure_paths() {
        let value = terminated(Path::new(r"C:\Program Files\Nickel\nickel.exe"));
        assert_eq!(value.last(), Some(&0));
        assert_eq!(string_length(&value), value.len() - 1);
        assert_eq!(string_length(&[b'N' as u16, 0, b'X' as u16]), 1);
    }

    #[test]
    fn installed_shortcut_icon_has_visible_pixels() {
        let Some(program_data) = std::env::var_os("PROGRAMDATA") else {
            return;
        };
        let root =
            std::path::PathBuf::from(program_data).join("Microsoft/Windows/Start Menu/Programs");
        let shortcut = [
            root.join("Google Chrome.lnk"),
            root.join("Windows Kits/Application Verifier (X64)/Application Verifier (X64).lnk"),
        ]
        .into_iter()
        .find(|path| path.is_file());
        let Some(shortcut) = shortcut else {
            return;
        };

        let image = path_icon_at_size(&shortcut, 48).expect("resolve an installed shortcut icon");
        assert!(image.pixels().any(|pixel| pixel.0[3] != 0));
        assert!(
            shortcut_target(&shortcut).is_some_and(|target| target.is_absolute()),
            "installed shortcut should expose an executable target"
        );
    }
}
