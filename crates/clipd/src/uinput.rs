use std::ffi::{c_int, c_ulong};
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::os::fd::AsRawFd;
use std::time::Duration;

const UI_SET_EVBIT: c_ulong = 0x4004_5564;
const UI_SET_KEYBIT: c_ulong = 0x4004_5565;
const UI_DEV_SETUP: c_ulong = 0x405c_5503;
const UI_DEV_CREATE: c_ulong = 0x5501;
const UI_DEV_DESTROY: c_ulong = 0x5502;

const EV_SYN: u16 = 0;
const EV_KEY: u16 = 1;
const SYN_REPORT: u16 = 0;

const KEY_LEFTCTRL: u16 = 29;
const KEY_V: u16 = 47;
const BUS_USB: u16 = 0x03;

extern "C" {
    fn ioctl(fd: c_int, request: c_ulong, ...) -> c_int;
}

#[repr(C)]
struct InputId {
    bustype: u16,
    vendor: u16,
    product: u16,
    version: u16,
}

#[repr(C)]
struct UinputSetup {
    id: InputId,
    name: [u8; 80],
    ff_effects_max: u32,
}

#[repr(C)]
struct InputEvent {
    tv_sec: i64,
    tv_usec: i64,
    type_: u16,
    code: u16,
    value: i32,
}

pub struct VirtualKeyboard {
    file: File,
}

impl VirtualKeyboard {
    pub fn open() -> Result<Self, String> {
        let file = OpenOptions::new()
            .write(true)
            .open("/dev/uinput")
            .map_err(|e| format!("open /dev/uinput: {}", e))?;

        let fd = file.as_raw_fd();

        unsafe {
            if ioctl(fd, UI_SET_EVBIT, EV_KEY as c_int) < 0 {
                return Err("ioctl UI_SET_EVBIT(EV_KEY) failed".to_string());
            }
            if ioctl(fd, UI_SET_EVBIT, EV_SYN as c_int) < 0 {
                return Err("ioctl UI_SET_EVBIT(EV_SYN) failed".to_string());
            }
            if ioctl(fd, UI_SET_KEYBIT, KEY_LEFTCTRL as c_int) < 0 {
                return Err("ioctl UI_SET_KEYBIT(KEY_LEFTCTRL) failed".to_string());
            }
            if ioctl(fd, UI_SET_KEYBIT, KEY_V as c_int) < 0 {
                return Err("ioctl UI_SET_KEYBIT(KEY_V) failed".to_string());
            }

            let mut setup = UinputSetup {
                id: InputId {
                    bustype: BUS_USB,
                    vendor: 0x1234,
                    product: 0x5678,
                    version: 1,
                },
                name: [0u8; 80],
                ff_effects_max: 0,
            };
            let label = b"clipboard-lx virtual keyboard";
            setup.name[..label.len()].copy_from_slice(label);

            if ioctl(fd, UI_DEV_SETUP, &setup as *const UinputSetup) < 0 {
                return Err("ioctl UI_DEV_SETUP failed".to_string());
            }
            if ioctl(fd, UI_DEV_CREATE) < 0 {
                return Err("ioctl UI_DEV_CREATE failed".to_string());
            }
        }

        Ok(Self { file })
    }

    pub fn send_ctrl_v(&mut self) -> Result<(), String> {
        self.emit(EV_KEY, KEY_LEFTCTRL, 1)?;
        self.emit(EV_SYN, SYN_REPORT, 0)?;
        std::thread::sleep(Duration::from_millis(12));

        self.emit(EV_KEY, KEY_V, 1)?;
        self.emit(EV_SYN, SYN_REPORT, 0)?;
        std::thread::sleep(Duration::from_millis(15));

        self.emit(EV_KEY, KEY_V, 0)?;
        self.emit(EV_SYN, SYN_REPORT, 0)?;
        std::thread::sleep(Duration::from_millis(12));

        self.emit(EV_KEY, KEY_LEFTCTRL, 0)?;
        self.emit(EV_SYN, SYN_REPORT, 0)?;
        Ok(())
    }

    fn emit(&mut self, type_: u16, code: u16, value: i32) -> Result<(), String> {
        let ev = InputEvent {
            tv_sec: 0,
            tv_usec: 0,
            type_,
            code,
            value,
        };
        let bytes = unsafe {
            std::slice::from_raw_parts(
                (&ev as *const InputEvent) as *const u8,
                std::mem::size_of::<InputEvent>(),
            )
        };
        self.file
            .write_all(bytes)
            .map_err(|e| format!("uinput write failed: {}", e))?;
        Ok(())
    }
}

impl Drop for VirtualKeyboard {
    fn drop(&mut self) {
        unsafe {
            let _ = ioctl(self.file.as_raw_fd(), UI_DEV_DESTROY);
        }
    }
}
