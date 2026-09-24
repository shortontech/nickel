//! Windows common item dialog for wallpaper image selection.

use crate::FileDialogOutcome;
use std::{ffi::c_void, path::PathBuf, thread};
use windows::{
    Win32::{
        Foundation::HWND,
        System::Com::{
            CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
            CoTaskMemFree, CoUninitialize,
        },
        System::Threading::GetCurrentProcessId,
        UI::Shell::{
            Common::COMDLG_FILTERSPEC, FOS_FILEMUSTEXIST, FOS_FORCEFILESYSTEM, FileOpenDialog,
            IFileOpenDialog, SIGDN_FILESYSPATH,
        },
        UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId},
    },
    core::w,
};

pub(crate) fn choose_image_file(
    callback: Box<dyn Fn(FileDialogOutcome) + Send + 'static>,
) -> Result<(), String> {
    // The click that opens the chooser makes Settings foreground. Only use a
    // window in our process as owner so the dialog stays above Settings.
    let owner = unsafe {
        let hwnd = GetForegroundWindow();
        let mut process_id = 0;
        if !hwnd.is_invalid()
            && GetWindowThreadProcessId(hwnd, Some(&mut process_id)) != 0
            && process_id == GetCurrentProcessId()
        {
            Some(hwnd.0 as usize)
        } else {
            None
        }
    };
    thread::Builder::new()
        .name("nickel-windows-image-chooser".into())
        .spawn(move || callback(request_image_file(owner)))
        .map(|_| ())
        .map_err(|error| format!("could not start the Windows image chooser: {error}"))
}

fn request_image_file(owner: Option<usize>) -> FileDialogOutcome {
    request_image_file_inner(owner).unwrap_or_else(FileDialogOutcome::Failed)
}

fn request_image_file_inner(owner: Option<usize>) -> Result<FileDialogOutcome, String> {
    // SAFETY: This dedicated thread has no prior COM apartment. The guard
    // uninitializes COM after all dialog and shell item references are dropped.
    unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }
        .ok()
        .map_err(|error| format!("initialize image chooser COM apartment: {error}"))?;
    let apartment = ComApartment;
    let outcome = show_image_dialog(owner.map(|raw| HWND(raw as *mut c_void)));
    drop(apartment);
    outcome
}

struct ComApartment;

impl Drop for ComApartment {
    fn drop(&mut self) {
        // SAFETY: Paired with this thread's successful CoInitializeEx.
        unsafe { CoUninitialize() };
    }
}

fn show_image_dialog(owner: Option<HWND>) -> Result<FileDialogOutcome, String> {
    // SAFETY: The caller initialized an STA, and Windows owns this COM class.
    let dialog: IFileOpenDialog =
        unsafe { CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER) }
            .map_err(|error| format!("create image chooser: {error}"))?;
    let filters = [COMDLG_FILTERSPEC {
        pszName: w!("Image files"),
        pszSpec: w!("*.png;*.jpg;*.jpeg;*.webp;*.bmp"),
    }];
    // SAFETY: The dialog retains the filters for Show; static wide strings
    // remain valid throughout the dialog's lifetime.
    unsafe {
        dialog
            .SetFileTypes(&filters)
            .map_err(|error| format!("configure image file types: {error}"))?;
        dialog
            .SetTitle(w!("Choose an image"))
            .map_err(|error| format!("set image chooser title: {error}"))?;
        let options = dialog
            .GetOptions()
            .map_err(|error| format!("read image chooser options: {error}"))?;
        dialog
            .SetOptions(options | FOS_FILEMUSTEXIST | FOS_FORCEFILESYSTEM)
            .map_err(|error| format!("configure image chooser options: {error}"))?;
        match dialog.Show(owner) {
            Ok(()) => {}
            Err(error) if error.code().0 == 0x8007_04c7_u32 as i32 => {
                return Ok(FileDialogOutcome::Cancelled);
            }
            Err(error) => return Err(format!("show image chooser: {error}")),
        }
        let item = dialog
            .GetResult()
            .map_err(|error| format!("read selected image: {error}"))?;
        let display_name = item
            .GetDisplayName(SIGDN_FILESYSPATH)
            .map_err(|error| format!("read selected image path: {error}"))?;
        let path = display_name.to_string();
        // SAFETY: GetDisplayName allocates this string with the COM task allocator.
        CoTaskMemFree(Some(display_name.0.cast()));
        path.map(PathBuf::from)
            .map(FileDialogOutcome::Selected)
            .map_err(|error| format!("decode selected image path: {error}"))
    }
}
