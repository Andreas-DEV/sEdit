//! En lille besked-dialog i appens egne farver.
//!
//! Windows' MessageBox kan ikke tegnes mørk, så spørgsmål som "Vil du gemme
//! ændringerne?" vises i dette vindue i stedet. Det opfører sig som MessageBox:
//! det blokerer ejervinduet og returnerer IDYES, IDNO, IDCANCEL eller IDOK.

use std::mem::{size_of, zeroed};
use std::ptr::{null, null_mut};
use std::sync::atomic::{AtomicBool, Ordering};

use windows_sys::w;
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{
    BeginPaint, BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, DeleteObject,
    DrawTextW, EndPaint, GetDC, InvalidateRect, ReleaseDC, SelectObject, SetBkMode, SetTextColor,
    DT_CALCRECT, DT_CENTER, DT_EDITCONTROL, DT_NOPREFIX, DT_SINGLELINE, DT_VCENTER, DT_WORDBREAK,
    PAINTSTRUCT, SRCCOPY, TRANSPARENT,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::HiDpi::{AdjustWindowRectExForDpi, GetDpiForSystem, GetDpiForWindow};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    EnableWindow, GetKeyState, SetActiveWindow, SetFocus, VK_ESCAPE, VK_LEFT, VK_RETURN, VK_RIGHT,
    VK_SHIFT, VK_SPACE, VK_TAB,
};
use windows_sys::Win32::UI::WindowsAndMessaging::*;

use crate::draw::{field, fill, mix};
use crate::lang;
use crate::ui;
use crate::util::{hi, lo, scale};

const CLASS: *const u16 = w!("sEdit.Dialog");

struct Button {
    id: i32,
    label: Vec<u16>,
    /// Bogstavtast der vælger knappen (J for Ja osv.).
    key: u8,
    rect: RECT,
}

struct Dialog {
    text: Vec<u16>,
    text_rect: RECT,
    buttons: Vec<Button>,
    /// Y-koordinat hvor knapbåndet begynder.
    band_top: i32,
    focus: usize,
    hover: Option<usize>,
    pressed: Option<usize>,
    /// Svaret på Esc og lukkeknappen; None når dialogen kræver et valg (Ja/Nej).
    cancel: Option<i32>,
    result: Option<i32>,
    dpi: u32,
}

/// En knap i dialogen: svaret den giver og teksten på den. Et & i teksten står foran
/// bogstavtasten der vælger knappen.
pub struct Choice {
    pub id: i32,
    pub label: &'static str,
}

/// Viser dialogen over `owner` og venter på svaret. `flags` er de samme som til MessageBox.
pub unsafe fn show(owner: HWND, text: &str, flags: u32) -> i32 {
    let words = lang::s();
    let yes = Choice { id: IDYES, label: words.yes };
    let no = Choice { id: IDNO, label: words.no };
    match flags & 0xF {
        MB_YESNOCANCEL => choose(owner, text, &[yes, no, Choice { id: IDCANCEL, label: words.cancel }], 0, Some(IDCANCEL)),
        MB_YESNO => choose(owner, text, &[yes, no], 0, None),
        _ => choose(owner, text, &[Choice { id: IDOK, label: words.ok }], 0, Some(IDOK)),
    }
}

/// Viser dialogen med egne knapper. `focus` er knappen Enter vælger fra start, og
/// `cancel` er svaret på Esc og lukkeknappen (None: dialogen kræver et valg).
pub unsafe fn choose(owner: HWND, text: &str, choices: &[Choice], focus: usize, cancel: Option<i32>) -> i32 {
    static REGISTERED: AtomicBool = AtomicBool::new(false);
    let hinst = GetModuleHandleW(null());
    if !REGISTERED.swap(true, Ordering::Relaxed) {
        let mut class: WNDCLASSEXW = zeroed();
        class.cbSize = size_of::<WNDCLASSEXW>() as u32;
        class.lpfnWndProc = Some(dialog_proc);
        class.hInstance = hinst;
        class.hCursor = LoadCursorW(null_mut(), IDC_ARROW);
        class.lpszClassName = CLASS;
        RegisterClassExW(&class);
    }

    let buttons: Vec<Button> = choices
        .iter()
        .map(|c| Button { id: c.id, label: lang::plain(c.label).encode_utf16().collect(), key: lang::mnemonic(c.label), rect: zeroed() })
        .collect();
    let dpi = if owner.is_null() { GetDpiForSystem() } else { GetDpiForWindow(owner) };
    let mut state = Dialog {
        text: text.encode_utf16().collect(),
        text_rect: zeroed(),
        buttons,
        band_top: 0,
        focus: focus.min(choices.len().saturating_sub(1)),
        hover: None,
        pressed: None,
        cancel,
        result: None,
        dpi,
    };
    let (width, height) = layout(&mut state, owner);

    // Uden lukkeknap når der ikke findes et "fortryd"-svar, præcis som MessageBox.
    let style = WS_POPUP | WS_CAPTION | if cancel.is_some() { WS_SYSMENU } else { 0 };
    let mut frame = RECT { left: 0, top: 0, right: width, bottom: height };
    AdjustWindowRectExForDpi(&mut frame, style, 0, WS_EX_DLGMODALFRAME, dpi);
    let (outer_w, outer_h) = (frame.right - frame.left, frame.bottom - frame.top);
    let mut over: RECT = zeroed();
    if owner.is_null() || IsIconic(owner) != 0 || GetWindowRect(owner, &mut over) == 0 {
        over = RECT { left: 0, top: 0, right: GetSystemMetrics(SM_CXSCREEN), bottom: GetSystemMetrics(SM_CYSCREEN) };
    }
    let x = over.left + (over.right - over.left - outer_w) / 2;
    let y = over.top + (over.bottom - over.top - outer_h) / 2;

    let hwnd = CreateWindowExW(
        WS_EX_DLGMODALFRAME,
        CLASS,
        w!("sEdit"),
        style,
        x,
        y,
        outer_w,
        outer_h,
        owner,
        null_mut(),
        hinst,
        null(),
    );
    if hwnd.is_null() {
        // Kan vinduet ikke oprettes, vælges det svar der ikke ændrer noget.
        return cancel.unwrap_or(choices[state.focus].id);
    }
    SetWindowLongPtrW(hwnd, GWLP_USERDATA, &mut state as *mut Dialog as isize);
    ui::apply_frame_theme(hwnd);

    if !owner.is_null() {
        EnableWindow(owner, 0);
    }
    ShowWindow(hwnd, SW_SHOW);
    SetFocus(hwnd);

    let mut msg: MSG = zeroed();
    while state.result.is_none() {
        let got = GetMessageW(&mut msg, null_mut(), 0, 0);
        if got <= 0 {
            // Appen er ved at lukke: giv beskeden videre til hovedløkken og svar "fortryd".
            if got == 0 {
                PostQuitMessage(msg.wParam as i32);
            }
            break;
        }
        TranslateMessage(&msg);
        DispatchMessageW(&msg);
    }

    // Ejeren aktiveres igen, før dialogen forsvinder; ellers giver Windows fokus til et andet program.
    if !owner.is_null() {
        EnableWindow(owner, 1);
        SetActiveWindow(owner);
    }
    SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
    DestroyWindow(hwnd);
    state.result.or(cancel).unwrap_or(choices[choices.len() - 1].id)
}

/// Måler teksten og placerer knapperne. Returnerer klientområdets størrelse.
unsafe fn layout(d: &mut Dialog, owner: HWND) -> (i32, i32) {
    let dpi = d.dpi;
    let pad = scale(dpi, 20);
    let (button_h, gap) = (scale(dpi, 30), scale(dpi, 8));
    let band_pad = scale(dpi, 12);

    let hdc = GetDC(owner);
    let previous = SelectObject(hdc, ui::ui_font(dpi));
    let mut measured = RECT { left: 0, top: 0, right: scale(dpi, 420), bottom: 0 };
    DrawTextW(hdc, d.text.as_ptr(), d.text.len() as i32, &mut measured, DT_CALCRECT | TEXT_FORMAT);
    // Knapperne er lige brede og brede nok til den længste tekst på det aktuelle sprog.
    let mut button_w = scale(dpi, 100);
    for b in &d.buttons {
        let mut label = RECT { left: 0, top: 0, right: 0, bottom: 0 };
        DrawTextW(hdc, b.label.as_ptr(), b.label.len() as i32, &mut label, DT_CALCRECT | DT_SINGLELINE | DT_NOPREFIX);
        button_w = button_w.max(label.right + scale(dpi, 28));
    }
    SelectObject(hdc, previous);
    ReleaseDC(owner, hdc);

    let count = d.buttons.len() as i32;
    let buttons_w = count * button_w + (count - 1) * gap;
    let width = (measured.right + 2 * pad).max(buttons_w + 2 * pad).max(scale(dpi, 320));
    d.text_rect = RECT { left: pad, top: pad, right: width - pad, bottom: pad + measured.bottom };
    d.band_top = d.text_rect.bottom + pad;
    let height = d.band_top + button_h + 2 * band_pad;

    let mut x = width - pad - buttons_w;
    for b in d.buttons.iter_mut() {
        b.rect = RECT { left: x, top: d.band_top + band_pad, right: x + button_w, bottom: d.band_top + band_pad + button_h };
        x += button_w + gap;
    }
    (width, height)
}

/// DT_EDITCONTROL deler også lange ord, så en lang filsti ikke stikker ud over kanten.
const TEXT_FORMAT: u32 = DT_WORDBREAK | DT_EDITCONTROL | DT_NOPREFIX;

unsafe fn paint(hwnd: HWND, d: &Dialog) {
    let mut ps: PAINTSTRUCT = zeroed();
    let hdc = BeginPaint(hwnd, &mut ps);
    let mut rc: RECT = zeroed();
    GetClientRect(hwnd, &mut rc);
    let theme = ui::theme();

    let mem = CreateCompatibleDC(hdc);
    let bitmap = CreateCompatibleBitmap(hdc, rc.right.max(1), rc.bottom.max(1));
    let old_bitmap = SelectObject(mem, bitmap);
    let old_font = SelectObject(mem, ui::ui_font(d.dpi));

    // Én baggrund med en tynd linje over knapperne, som i indstillingerne.
    fill(mem, &rc, theme.editor_bg);
    fill(mem, &RECT { top: d.band_top, bottom: d.band_top + scale(d.dpi, 1).max(1), ..rc }, theme.sep);
    SetBkMode(mem, TRANSPARENT as _);
    SetTextColor(mem, theme.text);
    let mut text_rect = d.text_rect;
    DrawTextW(mem, d.text.as_ptr(), d.text.len() as i32, &mut text_rect, TEXT_FORMAT);

    for (i, b) in d.buttons.iter().enumerate() {
        // Knappen med fokus er den Enter vælger, og den får accentfarven.
        let focused = i == d.focus;
        let pressed = d.pressed == Some(i);
        let hot = d.hover == Some(i) || pressed;
        if focused {
            let back = mix(theme.accent, theme.editor_bg, if pressed { 30 } else if hot { 15 } else { 0 });
            field(mem, &b.rect, d.dpi, back, back);
        } else {
            field(mem, &b.rect, d.dpi, if pressed { theme.close_hover } else if hot { theme.hover } else { theme.strip }, theme.sep);
        }
        SetTextColor(mem, if focused { theme.on_accent } else { theme.text });
        let mut label_rect = b.rect;
        DrawTextW(mem, b.label.as_ptr(), b.label.len() as i32, &mut label_rect, DT_SINGLELINE | DT_VCENTER | DT_CENTER | DT_NOPREFIX);
    }

    BitBlt(hdc, 0, 0, rc.right, rc.bottom, mem, 0, 0, SRCCOPY);
    SelectObject(mem, old_font);
    SelectObject(mem, old_bitmap);
    DeleteObject(bitmap);
    DeleteDC(mem);
    EndPaint(hwnd, &ps);
}

fn button_at(d: &Dialog, x: i32, y: i32) -> Option<usize> {
    d.buttons.iter().position(|b| x >= b.rect.left && x < b.rect.right && y >= b.rect.top && y < b.rect.bottom)
}

unsafe extern "system" fn dialog_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    let state = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Dialog;
    if state.is_null() {
        return DefWindowProcW(hwnd, msg, wp, lp);
    }
    let d = &mut *state;
    match msg {
        WM_PAINT => {
            paint(hwnd, d);
            0
        }
        WM_ERASEBKGND => 1,
        WM_MOUSEMOVE => {
            let over = button_at(d, lo(lp), hi(lp));
            if over != d.hover {
                d.hover = over;
                InvalidateRect(hwnd, null(), 0);
            }
            0
        }
        WM_LBUTTONDOWN => {
            d.pressed = button_at(d, lo(lp), hi(lp));
            if let Some(i) = d.pressed {
                d.focus = i;
            }
            InvalidateRect(hwnd, null(), 0);
            0
        }
        WM_LBUTTONUP => {
            let released = button_at(d, lo(lp), hi(lp));
            if released.is_some() && released == d.pressed.take() {
                d.result = released.map(|i| d.buttons[i].id);
            }
            InvalidateRect(hwnd, null(), 0);
            0
        }
        WM_KEYDOWN => {
            let key = wp as u16;
            let count = d.buttons.len();
            let back = key == VK_LEFT || (key == VK_TAB && GetKeyState(VK_SHIFT as i32) < 0);
            if key == VK_RETURN || key == VK_SPACE {
                d.result = Some(d.buttons[d.focus].id);
            } else if key == VK_ESCAPE {
                d.result = d.cancel;
            } else if back {
                d.focus = (d.focus + count - 1) % count;
            } else if key == VK_TAB || key == VK_RIGHT {
                d.focus = (d.focus + 1) % count;
            } else if let Some(b) = d.buttons.iter().find(|b| b.key as u16 == key) {
                d.result = Some(b.id);
            }
            InvalidateRect(hwnd, null(), 0);
            0
        }
        WM_CLOSE => {
            d.result = d.cancel;
            0
        }
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}
