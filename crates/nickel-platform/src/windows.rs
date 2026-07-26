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
            BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateCompatibleDC, CreateDIBSection,
            DIB_RGB_COLORS, DeleteDC, DeleteObject, GetDC, HGDIOBJ, ReleaseDC, SelectObject,
        },
        Storage::FileSystem::FILE_FLAGS_AND_ATTRIBUTES,
        System::Com::{
            CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
            CoUninitialize, IPersistFile, STGM_READ,
        },
        UI::{
            Shell::{ExtractIconExW, IShellLinkW, SHFILEINFOW, SHGFI_ICON, SHGetFileInfoW},
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
    let initialized = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }.is_ok();
    let shortcut = shortcut_icon(path);
    let (image, resolver) = if shortcut.is_some() {
        (shortcut, "shortcut")
    } else if let Some(internet_shortcut) = internet_shortcut_icon(path) {
        (Some(internet_shortcut), "internet-shortcut")
    } else {
        (shell_path_icon(path), "shell-path")
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

fn internet_shortcut_icon(path: &Path) -> Option<RgbaImage> {
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
    extract_icon(&icon_path, icon_index).or_else(|| shell_path_icon(&icon_path))
}

fn shortcut_icon(path: &Path) -> Option<RgbaImage> {
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
        return shell_path_icon(&target);
    }
    let length = string_length(&location);
    if length == 0 {
        return None;
    }
    location.truncate(length + 1);
    let explicit_path =
        std::path::PathBuf::from(std::ffi::OsString::from_wide(&location[..length]));

    let Some(image) = extract_icon(&explicit_path, index) else {
        tracing::debug!(
            shortcut = %path.display(),
            icon = %explicit_path.display(),
            index,
            "indexed shortcut icon extraction failed; trying shell path"
        );
        return shell_path_icon(&explicit_path);
    };
    Some(image)
}

fn extract_icon(path: &Path, index: i32) -> Option<RgbaImage> {
    let wide = terminated(path);
    let mut icon = HICON::default();
    let count =
        unsafe { ExtractIconExW(PCWSTR(wide.as_ptr()), index, Some(&raw mut icon), None, 1) };
    if count == 0 || icon.0.is_null() {
        return None;
    }
    let image = render_icon(icon);
    unsafe {
        let _ = DestroyIcon(icon);
    }
    image
}

fn shell_path_icon(path: &Path) -> Option<RgbaImage> {
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
        let image = render_icon(info.hIcon);
        let _ = DestroyIcon(info.hIcon);
        image
    }
}

fn render_icon(icon: HICON) -> Option<RgbaImage> {
    const SIZE: u32 = 32;
    let info = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: SIZE as i32,
            biHeight: -(SIZE as i32),
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
            SIZE as i32,
            SIZE as i32,
            0,
            None,
            DI_NORMAL,
        )
        .is_ok();
        let mut rgba = vec![0_u8; (SIZE * SIZE * 4) as usize];
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
            .then(|| RgbaImage::from_raw(SIZE, SIZE, rgba))
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
