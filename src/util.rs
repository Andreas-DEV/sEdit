use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;

/// UTF-16 med afsluttende NUL, klar til Win32.
pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

pub fn wide_os(s: &OsStr) -> Vec<u16> {
    s.encode_wide().chain(Some(0)).collect()
}

/// Fortegnsbevarende lav/høj halvdel af en LPARAM (musekoordinater kan være negative).
pub fn lo(v: isize) -> i32 {
    (v & 0xFFFF) as i16 as i32
}

pub fn hi(v: isize) -> i32 {
    ((v >> 16) & 0xFFFF) as i16 as i32
}

pub fn make_lparam(x: i32, y: i32) -> isize {
    (((y as u16 as u32) << 16) | (x as u16 as u32)) as isize
}

pub const fn rgb(r: u8, g: u8, b: u8) -> u32 {
    r as u32 | (g as u32) << 8 | (b as u32) << 16
}

/// Skalerer et mål i 96-DPI-pixels til vinduets DPI.
pub fn scale(dpi: u32, v: i32) -> i32 {
    (v * dpi as i32 + 48) / 96
}
