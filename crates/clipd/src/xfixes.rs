use std::ffi::{c_char, c_int, c_long, c_ulong};
use std::ptr;
use std::time::Duration;

#[repr(C)]
struct Display {
    _private: [u8; 0],
}

#[repr(C)]
struct XErrorEvent {
    _type: c_int,
    _display: *mut Display,
    _resourceid: c_ulong,
    _serial: c_ulong,
    _error_code: u8,
    _request_code: u8,
    _minor_code: u8,
}

#[repr(C)]
union XEvent {
    event_type: c_int,
    pad: [c_long; 24],
}

const XFIXES_SET_SELECTION_OWNER_NOTIFY_MASK: c_ulong = 1 << 0;
const XFIXES_SELECTION_NOTIFY: c_int = 0;

type XErrorHandler = Option<unsafe extern "C" fn(*mut Display, *mut XErrorEvent) -> c_int>;

extern "C" {
    fn XOpenDisplay(display_name: *const c_char) -> *mut Display;
    fn XCloseDisplay(display: *mut Display) -> c_int;
    fn XDefaultRootWindow(display: *mut Display) -> c_ulong;
    fn XInternAtom(display: *mut Display, atom_name: *const c_char, only_if_exists: c_int) -> c_ulong;
    fn XNextEvent(display: *mut Display, event_return: *mut XEvent) -> c_int;
    fn XPending(display: *mut Display) -> c_int;
    fn XFlush(display: *mut Display) -> c_int;
    fn XSetErrorHandler(handler: XErrorHandler) -> XErrorHandler;

    fn XFixesQueryExtension(
        display: *mut Display,
        event_base_return: *mut c_int,
        error_base_return: *mut c_int,
    ) -> c_int;
    fn XFixesSelectSelectionInput(
        display: *mut Display,
        win: c_ulong,
        selection: c_ulong,
        event_mask: c_ulong,
    );
}

unsafe extern "C" fn ignore_x_error(_dpy: *mut Display, _err: *mut XErrorEvent) -> c_int {
    0
}

/// Blocks on the dedicated monitoring thread and invokes `on_change` whenever
/// XFixes reports a new `CLIPBOARD` owner (zero polling).
pub fn run_xfixes_monitor<F>(mut on_change: F)
where
    F: FnMut(),
{
    unsafe {
        XSetErrorHandler(Some(ignore_x_error));
    }

    let mut backoff_ms = 250u64;

    loop {
        let dpy = unsafe { XOpenDisplay(ptr::null()) };
        if dpy.is_null() {
            eprintln!(
                "clipd: XOpenDisplay failed (waiting {} ms before retry)",
                backoff_ms
            );
            std::thread::sleep(Duration::from_millis(backoff_ms));
            backoff_ms = (backoff_ms * 2).min(5000);
            continue;
        }
        backoff_ms = 250;

        let mut event_base: c_int = 0;
        let mut error_base: c_int = 0;
        let has_xfixes = unsafe { XFixesQueryExtension(dpy, &mut event_base, &mut error_base) };
        if has_xfixes == 0 {
            eprintln!("clipd: XFixes extension unavailable on X display");
            unsafe { XCloseDisplay(dpy) };
            std::thread::sleep(Duration::from_secs(5));
            continue;
        }

        let root = unsafe { XDefaultRootWindow(dpy) };
        let clipboard_atom = unsafe { XInternAtom(dpy, c"CLIPBOARD".as_ptr(), 0) };
        if clipboard_atom == 0 {
            eprintln!("clipd: failed to intern CLIPBOARD atom");
            unsafe { XCloseDisplay(dpy) };
            std::thread::sleep(Duration::from_secs(2));
            continue;
        }

        unsafe {
            XFixesSelectSelectionInput(
                dpy,
                root,
                clipboard_atom,
                XFIXES_SET_SELECTION_OWNER_NOTIFY_MASK,
            );
            XFlush(dpy);
        }

        let notify_type = event_base + XFIXES_SELECTION_NOTIFY;
        let mut ev = XEvent { pad: [0; 24] };

        loop {
            unsafe {
                XNextEvent(dpy, &mut ev);
            }
            let ev_type = unsafe { ev.event_type };
            if ev_type == notify_type {
                // Brief debounce so apps publishing multiple targets settle before we read.
                std::thread::sleep(Duration::from_millis(35));
                while unsafe { XPending(dpy) } > 0 {
                    unsafe {
                        XNextEvent(dpy, &mut ev);
                    }
                }
                on_change();
            }
        }
    }
}
