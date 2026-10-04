//! Test-only DLL: acknowledge injection before the fake game's main thread runs.
//! The rig test substitutes the event pattern from tpf3mp-ipc before compiling.
use std::ffi::c_void;

#[link(name = "kernel32")]
unsafe extern "system" {
    fn GetCurrentProcessId() -> u32;
    fn OpenEventW(access: u32, inherit: i32, name: *const u16) -> *mut c_void;
    fn SetEvent(event: *mut c_void) -> i32;
    fn CloseHandle(handle: *mut c_void) -> i32;
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn DllMain(_: *mut c_void, reason: u32, _: *mut c_void) -> i32 {
    if reason == 1 {
        // Only kernel32 event operations; no DLL loads or waits under the loader lock.
        unsafe {
            let pid = GetCurrentProcessId();
            let name: Vec<u16> = format!(__EVENT_PATTERN__)
                .encode_utf16()
                .chain(Some(0))
                .collect();
            let event = OpenEventW(2, 0, name.as_ptr());
            if event.is_null() {
                return 0;
            }
            let signalled = SetEvent(event);
            CloseHandle(event);
            return signalled;
        }
    }
    1
}
