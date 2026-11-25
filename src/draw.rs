//! Tegnehjælpere som vinduerne i appens egne farver deler: flader, afrundede felter og farveblanding.

use std::ptr::{null, null_mut};

use windows_sys::Win32::Foundation::RECT;
use windows_sys::Win32::Graphics::Gdi::{
    CreateCompatibleBitmap, CreateCompatibleDC, CreateSolidBrush, DeleteDC, DeleteObject, ExtTextOutW,
    GetStockObject, RoundRect, SelectObject, SetBkColor, SetBrushOrgEx, SetStretchBltMode, StretchBlt,
    ETO_OPAQUE, HALFTONE, HDC, NULL_PEN, SRCCOPY,
};

use crate::ui;
use crate::util::scale;

/// Hjørnernes runding på felter og knapper.
pub const RADIUS: i32 = 4;

pub unsafe fn fill(hdc: HDC, r: &RECT, color: u32) {
    SetBkColor(hdc, color);
    ExtTextOutW(hdc, 0, 0, ETO_OPAQUE, r, null(), 0, null());
}

/// En farve blandet med `percent` procent af en anden.
pub fn mix(color: u32, other: u32, percent: u32) -> u32 {
    let channel = |shift: u32| (((color >> shift) & 255) * (100 - percent) + ((other >> shift) & 255) * percent) / 100 << shift;
    channel(0) | channel(8) | channel(16)
}

/// En afrundet flade med bløde kanter. GDI kan ikke udjævne kanter selv, så fladen
/// tegnes i firedobbelt størrelse oven på farven bag den og skaleres ned.
pub unsafe fn round(hdc: HDC, r: &RECT, radius: i32, color: u32, behind: u32) {
    outlined(hdc, r, radius, color, color, 0, behind);
}

/// Som `round`, med en kant i sin egen farve, `line` punkter bred.
pub unsafe fn outlined(hdc: HDC, r: &RECT, radius: i32, color: u32, border: u32, line: i32, behind: u32) {
    const OVER: i32 = 4;
    let (width, height) = (r.right - r.left, r.bottom - r.top);
    if width <= 0 || height <= 0 {
        return;
    }
    let big = CreateCompatibleDC(hdc);
    let bitmap = CreateCompatibleBitmap(hdc, width * OVER, height * OVER);
    let old_bitmap = SelectObject(big, bitmap);
    fill(big, &RECT { left: 0, top: 0, right: width * OVER, bottom: height * OVER }, behind);
    let old_pen = SelectObject(big, GetStockObject(NULL_PEN));
    // Kanten er hele figuren i kantens farve med fladen tegnet oven på, lidt mindre.
    let shape = |shade: u32, edge: i32| {
        let brush = CreateSolidBrush(shade);
        let old_brush = SelectObject(big, brush);
        let corner = 2 * (radius * OVER - edge).max(0);
        // Uden pen stopper figuren et punkt før højre og nederste kant.
        RoundRect(big, edge, edge, width * OVER - edge + 1, height * OVER - edge + 1, corner, corner);
        SelectObject(big, old_brush);
        DeleteObject(brush);
    };
    if line > 0 && border != color {
        shape(border, 0);
    }
    shape(color, if border != color { line * OVER } else { 0 });
    SetStretchBltMode(hdc, HALFTONE);
    SetBrushOrgEx(hdc, 0, 0, null_mut());
    StretchBlt(hdc, r.left, r.top, width, height, big, 0, 0, width * OVER, height * OVER, SRCCOPY);
    SelectObject(big, old_pen);
    SelectObject(big, old_bitmap);
    DeleteObject(bitmap);
    DeleteDC(big);
}

/// Et afrundet felt med en tynd kant, som det står på vinduets baggrund.
pub unsafe fn field(hdc: HDC, r: &RECT, dpi: u32, color: u32, border: u32) {
    let radius = scale(dpi, RADIUS);
    let line = scale(dpi, 1).max(1);
    outlined(hdc, r, radius, color, border, line, ui::theme().editor_bg);
}
