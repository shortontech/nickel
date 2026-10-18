//! Narrow PAM authentication boundary for the compositor lock surface.

use std::{
    ffi::{CString, c_char, c_int, c_void},
    ptr,
};

use libloading::{Library, Symbol};
use zeroize::Zeroizing;

const PAM_SUCCESS: c_int = 0;
const PAM_PROMPT_ECHO_OFF: c_int = 1;
const PAM_PROMPT_ECHO_ON: c_int = 2;
const PAM_CONV_ERR: c_int = 19;

#[repr(C)]
struct PamMessage {
    msg_style: c_int,
    msg: *const c_char,
}

#[repr(C)]
struct PamResponse {
    resp: *mut c_char,
    resp_retcode: c_int,
}

type ConversationFn = unsafe extern "C" fn(
    c_int,
    *mut *const PamMessage,
    *mut *mut PamResponse,
    *mut c_void,
) -> c_int;

#[repr(C)]
struct PamConversation {
    conv: Option<ConversationFn>,
    appdata_ptr: *mut c_void,
}

struct ConversationData {
    username: CString,
    password: Zeroizing<Vec<u8>>,
}

unsafe extern "C" {
    fn calloc(count: usize, size: usize) -> *mut c_void;
    fn free(pointer: *mut c_void);
    fn strdup(value: *const c_char) -> *mut c_char;
}

unsafe extern "C" fn converse(
    count: c_int,
    messages: *mut *const PamMessage,
    responses: *mut *mut PamResponse,
    data: *mut c_void,
) -> c_int {
    if count <= 0 || messages.is_null() || responses.is_null() || data.is_null() {
        return PAM_CONV_ERR;
    }
    let count = count as usize;
    // SAFETY: PAM owns `messages` for `count` entries during this callback.
    let messages = unsafe { std::slice::from_raw_parts(messages, count) };
    // SAFETY: calloc returns C-allocator memory, which PAM expects for its
    // response array and releases after the conversation.
    let result = unsafe { calloc(count, std::mem::size_of::<PamResponse>()) }.cast::<PamResponse>();
    if result.is_null() {
        return PAM_CONV_ERR;
    }
    // SAFETY: `data` points to ConversationData for the duration of pam_authenticate.
    let data = unsafe { &*(data.cast::<ConversationData>()) };
    for (index, message) in messages.iter().enumerate() {
        if message.is_null() {
            unsafe { free_responses(result, index) };
            return PAM_CONV_ERR;
        }
        // SAFETY: each non-null entry is a PAM-owned PamMessage.
        let style = unsafe { (**message).msg_style };
        let answer = match style {
            PAM_PROMPT_ECHO_OFF => data.password.as_ptr().cast(),
            PAM_PROMPT_ECHO_ON => data.username.as_ptr(),
            _ => ptr::null(),
        };
        if !answer.is_null() {
            // SAFETY: answer is a valid NUL-terminated CString and strdup uses
            // the allocator PAM expects when it releases the response.
            let copy = unsafe { strdup(answer) };
            if copy.is_null() {
                unsafe { free_responses(result, index) };
                return PAM_CONV_ERR;
            }
            // SAFETY: result has count initialized entries.
            unsafe { (*result.add(index)).resp = copy };
        }
    }
    // SAFETY: caller supplied a valid out pointer for the response array.
    unsafe { *responses = result };
    PAM_SUCCESS
}

unsafe fn free_responses(responses: *mut PamResponse, initialized: usize) {
    for index in 0..initialized {
        // SAFETY: indices below initialized contain either null or strdup memory.
        let response = unsafe { (*responses.add(index)).resp };
        if !response.is_null() {
            // SAFETY: response came from strdup.
            unsafe { free(response.cast()) };
        }
    }
    // SAFETY: responses came from calloc.
    unsafe { free(responses.cast()) };
}

type PamStart = unsafe extern "C" fn(
    *const c_char,
    *const c_char,
    *const PamConversation,
    *mut *mut c_void,
) -> c_int;
type PamAuthenticate = unsafe extern "C" fn(*mut c_void, c_int) -> c_int;
type PamAccount = unsafe extern "C" fn(*mut c_void, c_int) -> c_int;
type PamEnd = unsafe extern "C" fn(*mut c_void, c_int) -> c_int;

pub fn authenticate(username: &str, password: &Zeroizing<String>) -> Result<bool, String> {
    let service = CString::new("login").expect("static PAM service has no NUL");
    let username = CString::new(username).map_err(|_| "username contains a NUL byte")?;
    if password.as_bytes().contains(&0) {
        return Err("password contains a NUL byte".into());
    }
    let mut password_bytes = password.as_bytes().to_vec();
    password_bytes.push(0);
    let password = Zeroizing::new(password_bytes);
    let mut data = ConversationData { username, password };
    let conversation = PamConversation {
        conv: Some(converse),
        appdata_ptr: (&mut data as *mut ConversationData).cast(),
    };

    // SAFETY: the symbols below retain this library for their entire use.
    let library = unsafe { Library::new("libpam.so.0") }.map_err(|error| error.to_string())?;
    unsafe {
        let start: Symbol<'_, PamStart> = library.get(b"pam_start\0").map_err(symbol_error)?;
        let authenticate: Symbol<'_, PamAuthenticate> =
            library.get(b"pam_authenticate\0").map_err(symbol_error)?;
        let account: Symbol<'_, PamAccount> =
            library.get(b"pam_acct_mgmt\0").map_err(symbol_error)?;
        let end: Symbol<'_, PamEnd> = library.get(b"pam_end\0").map_err(symbol_error)?;
        let mut handle = ptr::null_mut();
        let started = start(
            service.as_ptr(),
            data.username.as_ptr(),
            &conversation,
            &mut handle,
        );
        if started != PAM_SUCCESS {
            return Ok(false);
        }
        let authenticated = authenticate(handle, 0);
        let result = if authenticated == PAM_SUCCESS {
            account(handle, 0)
        } else {
            authenticated
        };
        let _ = end(handle, result);
        Ok(result == PAM_SUCCESS)
    }
}

fn symbol_error(error: libloading::Error) -> String {
    error.to_string()
}
