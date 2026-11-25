//! Indstillinger: værdierne, hvor de gemmes, og vinduet man retter dem i.
//!
//! Alt gemmes som tal under HKCU\Software\sEdit, og en ændring virker med det samme;
//! der er ingen OK-knap. Vinduet er tegnet i appens egne farver ligesom `dialog`,
//! fordi Windows' egne kontroller ikke kan tegnes mørke.

use std::mem::{size_of, zeroed};
use std::ptr::{null, null_mut};
use std::sync::atomic::{AtomicBool, Ordering};

use windows_sys::w;
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{
    BeginPaint, BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, CreateFontW, DeleteDC,
    DeleteObject, DrawTextW, EndPaint, GetDC, GetTextFaceW, InvalidateRect, ReleaseDC, SelectObject,
    SetBkMode, SetTextColor, DT_CALCRECT, DT_CENTER, DT_END_ELLIPSIS, DT_NOPREFIX, DT_SINGLELINE,
    DT_VCENTER, HDC, HFONT, PAINTSTRUCT, SRCCOPY, TRANSPARENT,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::HiDpi::{AdjustWindowRectExForDpi, GetDpiForSystem, GetDpiForWindow};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    EnableWindow, GetKeyState, MapVirtualKeyW, SetActiveWindow, SetFocus, VK_BACK, VK_CONTROL,
    VK_DELETE, VK_ESCAPE, VK_F1, VK_F24, VK_F4, VK_LWIN, VK_MENU, VK_RETURN, VK_RWIN, VK_SHIFT,
};
use windows_sys::Win32::UI::WindowsAndMessaging::*;

use crate::ui::{
    self, Change, CMD_CLOSE, CMD_FIND, CMD_FIND_NEXT, CMD_FIND_PREV, CMD_NEW, CMD_NEXT, CMD_OPEN, CMD_PREV, CMD_SAVE,
    CMD_SAVE_AS, CMD_SETTINGS,
    CMD_SPACE_COMMENTS, CMD_WRAP,
};
use crate::draw::{field, fill, mix, outlined, round, RADIUS};
use crate::lang;
use crate::util::{hi, lo, scale, wide};

// ---------- taster ----------

/// En tastkombination: tastens virtuelle kode i de lave 16 bit og Ctrl/Alt/Shift over
/// dem. 0 er "ingen genvej".
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Key(pub u32);

impl Key {
    const CTRL: u32 = 1 << 16;
    const ALT: u32 = 1 << 17;
    const SHIFT: u32 = 1 << 18;
    pub const NONE: Key = Key(0);

    pub const fn new(vk: u16, ctrl: bool, alt: bool, shift: bool) -> Key {
        Key(vk as u32
            | if ctrl { Key::CTRL } else { 0 }
            | if alt { Key::ALT } else { 0 }
            | if shift { Key::SHIFT } else { 0 })
    }

    const fn ctrl(vk: u16) -> Key {
        Key::new(vk, true, false, false)
    }

    pub fn vk(self) -> u16 {
        self.0 as u16
    }

    fn has(self, flag: u32) -> bool {
        self.0 & flag != 0
    }

    fn is_function_key(self) -> bool {
        (VK_F1..=VK_F24).contains(&self.vk())
    }

    /// Hvorfor kombinationen ikke kan bruges som genvej, eller None hvis den kan.
    pub fn problem(self) -> Option<&'static str> {
        let (ctrl, alt) = (self.has(Key::CTRL), self.has(Key::ALT));
        let vk = self.vk();
        if !ctrl && !alt && !self.is_function_key() {
            return Some(lang::s().need_modifier);
        }
        if ctrl && alt {
            // Ctrl+Alt er AltGr på dansk tastatur og skriver @, €, { og }.
            return Some(lang::s().no_ctrl_alt);
        }
        if alt && vk == VK_F4 {
            return Some(lang::s().no_alt_f4);
        }
        if ctrl && !self.has(Key::SHIFT) {
            if b"CXVZYA".iter().any(|&c| vk == c as u16) {
                return Some(lang::s().reserved);
            }
            if (b'1' as u16..=b'9' as u16).contains(&vk) {
                return Some(lang::s().no_ctrl_digit);
            }
        }
        None
    }

    /// Kombinationen som den vises: "Ctrl+Shift+S". Tom for "ingen genvej".
    pub fn name(self) -> String {
        if self == Key::NONE {
            return String::new();
        }
        let mut name = String::new();
        for (flag, label) in [(Key::CTRL, "Ctrl+"), (Key::ALT, "Alt+"), (Key::SHIFT, "Shift+")] {
            if self.has(flag) {
                name.push_str(label);
            }
        }
        let vk = self.vk();
        let words = lang::s();
        let fixed = match vk {
            0x08 => "Backspace",
            0x09 => "Tab",
            0x0D => "Enter",
            0x20 => words.key_space,
            0x21 => "PgUp",
            0x22 => "PgDn",
            0x23 => "End",
            0x24 => "Home",
            0x25 => words.key_left,
            0x26 => words.key_up,
            0x27 => words.key_right,
            0x28 => words.key_down,
            0x2D => "Insert",
            0x2E => "Delete",
            _ => "",
        };
        if !fixed.is_empty() {
            name.push_str(fixed);
        } else if self.is_function_key() {
            name.push_str(&format!("F{}", vk - VK_F1 + 1));
        } else {
            // Tegnet på tasten i det aktuelle tastaturlayout (2 = MAPVK_VK_TO_CHAR).
            let character = unsafe { MapVirtualKeyW(vk as u32, 2) } & 0x7FFF;
            match char::from_u32(character).filter(|c| !c.is_control() && character != 0) {
                Some(c) => name.extend(c.to_uppercase()),
                None => name.push_str(&lang::fill(words.key_other, &[&vk.to_string()])),
            }
        }
        name
    }
}

/// En kommando der kan få en genvej.
pub struct Command {
    pub id: usize,
    /// Navnet i registreringsdatabasen.
    key: &'static str,
    /// Kommandoens navn på det aktuelle sprog.
    label: fn() -> &'static str,
    default: [Key; 2],
}

impl Command {
    pub fn label(&self) -> &'static str {
        (self.label)()
    }
}

const VK_TAB: u16 = 0x09;
const VK_PRIOR: u16 = 0x21;
const VK_NEXT: u16 = 0x22;
const VK_OEM_COMMA: u16 = 0xBC;

pub const COMMANDS: [Command; 13] = [
    Command { id: CMD_SAVE, key: "Save", label: || lang::s().save, default: [Key::ctrl(b'S' as u16), Key::NONE] },
    Command { id: CMD_SAVE_AS, key: "SaveAs", label: || lang::s().save_as, default: [Key::new(b'S' as u16, true, false, true), Key::NONE] },
    Command { id: CMD_NEW, key: "New", label: || lang::s().new_tab, default: [Key::ctrl(b'N' as u16), Key::ctrl(b'T' as u16)] },
    Command { id: CMD_OPEN, key: "Open", label: || lang::s().open_file, default: [Key::ctrl(b'O' as u16), Key::NONE] },
    Command { id: CMD_CLOSE, key: "Close", label: || lang::s().close_tab, default: [Key::ctrl(b'W' as u16), Key::ctrl(VK_F4)] },
    Command { id: CMD_NEXT, key: "Next", label: || lang::s().next_tab, default: [Key::ctrl(VK_TAB), Key::ctrl(VK_NEXT)] },
    Command {
        id: CMD_PREV,
        key: "Prev",
        label: || lang::s().prev_tab,
        default: [Key::new(VK_TAB, true, false, true), Key::ctrl(VK_PRIOR)],
    },
    Command { id: CMD_WRAP, key: "Wrap", label: || lang::s().wrap_lines, default: [Key::new(b'Z' as u16, false, true, false), Key::NONE] },
    Command {
        id: CMD_SPACE_COMMENTS,
        key: "SpaceComments",
        label: || lang::s().space_comments,
        default: [Key::new(b'A' as u16, false, true, false), Key::NONE],
    },
    Command { id: CMD_SETTINGS, key: "Settings", label: || lang::s().settings, default: [Key::ctrl(VK_OEM_COMMA), Key::NONE] },
    Command { id: CMD_FIND, key: "Find", label: || lang::s().find_replace, default: [Key::ctrl(b'F' as u16), Key::ctrl(b'H' as u16)] },
    Command { id: CMD_FIND_NEXT, key: "FindNext", label: || lang::s().find_next, default: [Key::new(VK_F1 + 2, false, false, false), Key::NONE] },
    Command { id: CMD_FIND_PREV, key: "FindPrev", label: || lang::s().find_prev, default: [Key::new(VK_F1 + 2, false, false, true), Key::NONE] },
];

// ---------- værdierne ----------

/// Skrifttyperne man kan vælge imellem. Kun dem der er installeret, vises.
pub const FONTS: [&str; 6] = ["Consolas", "Cascadia Mono", "Cascadia Code", "JetBrains Mono", "Courier New", "Lucida Console"];
const SIZES: std::ops::RangeInclusive<u32> = 8..=28;

#[derive(Clone, PartialEq)]
pub struct Settings {
    /// 0 følger Windows, 1 er lyst, 2 er mørkt.
    pub theme: u32,
    /// Indeks i `FONTS`.
    pub font: usize,
    /// Skriftstørrelse i punkter.
    pub size: u32,
    pub wrap: bool,
    pub syntax: bool,
    pub line_numbers: bool,
    /// Om søgningen kun finder hele ord.
    pub whole_word: bool,
    /// Tabulatorbredde i tegn.
    pub tab: u32,
    /// 0 følger Windows; ellers sprogets plads i sproglisten + 1.
    pub language: u32,
    /// To genveje pr. kommando, i samme rækkefølge som `COMMANDS`.
    pub keys: [[Key; 2]; COMMANDS.len()],
}

impl Default for Settings {
    fn default() -> Settings {
        Settings {
            theme: 0,
            font: 0,
            size: 11,
            wrap: false,
            syntax: true,
            line_numbers: false,
            whole_word: false,
            tab: 4,
            language: 0,
            keys: COMMANDS.each_ref().map(|c| c.default),
        }
    }
}

/// En gemt genvej får denne bit sat, så "ingen genvej" kan skelnes fra "aldrig gemt".
const STORED: u32 = 1 << 31;

impl Settings {
    pub unsafe fn load() -> Settings {
        let mut s = Settings::default();
        let read = |name: &str| ui::read_setting(wide(name).as_ptr());
        s.theme = read("Theme").filter(|&v| v < 3).unwrap_or(s.theme);
        s.font = read("Font").map(|v| v as usize).filter(|&v| v < FONTS.len()).unwrap_or(s.font);
        s.size = read("FontSize").filter(|v| SIZES.contains(v)).unwrap_or(s.size);
        s.wrap = read("WordWrap").map_or(s.wrap, |v| v == 1);
        s.syntax = read("Syntax").map_or(s.syntax, |v| v == 1);
        s.line_numbers = read("LineNumbers").map_or(s.line_numbers, |v| v == 1);
        s.whole_word = read("WholeWord").map_or(s.whole_word, |v| v == 1);
        s.tab = read("TabWidth").filter(|v| TABS_WIDTHS.contains(v)).unwrap_or(s.tab);
        s.language = read("Language").filter(|&v| v < lang::CHOICES).unwrap_or(s.language);
        for (command, keys) in COMMANDS.iter().zip(s.keys.iter_mut()) {
            for (slot, key) in keys.iter_mut().enumerate() {
                if let Some(stored) = read(&format!("Key.{}.{slot}", command.key)).filter(|v| v & STORED != 0) {
                    let candidate = Key(stored & !STORED);
                    if candidate == Key::NONE || candidate.problem().is_none() {
                        *key = candidate;
                    }
                }
            }
        }
        s
    }

    pub unsafe fn save(&self) {
        let write = |name: &str, value: u32| ui::write_setting(wide(name).as_ptr(), value);
        write("Theme", self.theme);
        write("Font", self.font as u32);
        write("FontSize", self.size);
        write("WordWrap", self.wrap as u32);
        write("Syntax", self.syntax as u32);
        write("LineNumbers", self.line_numbers as u32);
        write("WholeWord", self.whole_word as u32);
        write("TabWidth", self.tab);
        write("Language", self.language);
        for (command, keys) in COMMANDS.iter().zip(&self.keys) {
            for (slot, key) in keys.iter().enumerate() {
                write(&format!("Key.{}.{slot}", command.key), key.0 | STORED);
            }
        }
    }

    pub fn font_face(&self) -> &'static str {
        FONTS[self.font.min(FONTS.len() - 1)]
    }

    /// Kommandoen der hører til en tastkombination.
    pub fn command_for(&self, key: Key) -> Option<usize> {
        if key == Key::NONE {
            return None;
        }
        COMMANDS.iter().zip(&self.keys).find(|(_, keys)| keys.contains(&key)).map(|(command, _)| command.id)
    }

    /// Den første genvej til en kommando, som den vises i menuer.
    pub fn shortcut(&self, command: usize) -> String {
        COMMANDS
            .iter()
            .zip(&self.keys)
            .find(|(c, _)| c.id == command)
            .and_then(|(_, keys)| keys.iter().find(|k| **k != Key::NONE))
            .map_or(String::new(), |key| key.name())
    }

    /// Giver kommandoen en genvej. Bruges kombinationen allerede af en anden kommando,
    /// tages den fra den; returnerer navnet på den kommando.
    fn assign(&mut self, command: usize, slot: usize, key: Key) -> Option<&'static str> {
        let mut taken_from = None;
        if key != Key::NONE {
            for (index, keys) in self.keys.iter_mut().enumerate() {
                for (other, existing) in keys.iter_mut().enumerate() {
                    if *existing == key && (index, other) != (command, slot) {
                        *existing = Key::NONE;
                        if index != command {
                            taken_from = Some(COMMANDS[index].label());
                        }
                    }
                }
            }
        }
        self.keys[command][slot] = key;
        taken_from
    }
}

// ---------- vinduet ----------

const CLASS: *const u16 = w!("sEdit.Settings");
const GLYPH_LEFT: u16 = 0xE76B;
const GLYPH_RIGHT: u16 = 0xE76C;
const PAGES: usize = 2;
/// Antal rækker under Generelt.
const GENERAL: usize = 9;
/// De første rækker under Generelt har flere valg; resten er kontakter.
const STEPS: usize = 5;
const TABS_WIDTHS: [u32; 4] = [2, 3, 4, 8];

fn page_name(page: usize) -> &'static str {
    let words = lang::s();
    [words.general, words.shortcuts][page]
}

fn general_labels() -> [&'static str; GENERAL] {
    let w = lang::s();
    [w.theme, w.font, w.font_size, w.tab_width, w.language, w.wrap_long, w.color_code, w.line_numbers, w.whole_words]
}

/// Det man kan pege på i vinduet.
#[derive(Clone, Copy, PartialEq)]
enum Target {
    Page(usize),
    /// Pilene på en række med flere valg; `false` er venstre.
    Step(usize, bool),
    Toggle(usize),
    Shortcut(usize, usize),
    Reset,
    Close,
}

struct Panel {
    dpi: u32,
    page: usize,
    hover: Option<Target>,
    pressed: Option<Target>,
    /// Genvejen der venter på et tastetryk: (kommando, plads).
    capture: Option<(usize, usize)>,
    note: String,
    /// De af `FONTS` der er installeret.
    fonts: Vec<usize>,
    /// Bredden af hver fanes tekst på det aktuelle sprog.
    tabs: [i32; PAGES],
    done: bool,
}

/// Viser indstillingerne over `owner` og venter, til vinduet lukkes.
pub unsafe fn show(owner: HWND) {
    static REGISTERED: AtomicBool = AtomicBool::new(false);
    let hinst = GetModuleHandleW(null());
    if !REGISTERED.swap(true, Ordering::Relaxed) {
        let mut class: WNDCLASSEXW = zeroed();
        class.cbSize = size_of::<WNDCLASSEXW>() as u32;
        class.lpfnWndProc = Some(panel_proc);
        class.hInstance = hinst;
        class.hCursor = LoadCursorW(null_mut(), IDC_ARROW);
        class.lpszClassName = CLASS;
        RegisterClassExW(&class);
    }

    let dpi = if owner.is_null() { GetDpiForSystem() } else { GetDpiForWindow(owner) };
    let mut panel = Panel {
        dpi,
        page: 0,
        hover: None,
        pressed: None,
        capture: None,
        note: String::new(),
        fonts: installed_fonts(owner),
        tabs: tab_widths(owner, dpi),
        done: false,
    };
    let (width, height) = client_size(dpi);
    let style = WS_POPUP | WS_CAPTION | WS_SYSMENU;
    let mut frame = RECT { left: 0, top: 0, right: width, bottom: height };
    AdjustWindowRectExForDpi(&mut frame, style, 0, WS_EX_DLGMODALFRAME, dpi);
    let (outer_w, outer_h) = (frame.right - frame.left, frame.bottom - frame.top);
    let mut over: RECT = zeroed();
    if owner.is_null() || IsIconic(owner) != 0 || GetWindowRect(owner, &mut over) == 0 {
        over = RECT { left: 0, top: 0, right: GetSystemMetrics(SM_CXSCREEN), bottom: GetSystemMetrics(SM_CYSCREEN) };
    }
    let x = over.left + (over.right - over.left - outer_w) / 2;
    let y = (over.top + (over.bottom - over.top - outer_h) / 2).max(0);

    let hwnd = CreateWindowExW(WS_EX_DLGMODALFRAME, CLASS, wide(lang::s().settings).as_ptr(), style, x, y, outer_w, outer_h, owner, null_mut(), hinst, null());
    if hwnd.is_null() {
        return;
    }
    SetWindowLongPtrW(hwnd, GWLP_USERDATA, &mut panel as *mut Panel as isize);
    ui::apply_frame_theme(hwnd);
    if !owner.is_null() {
        EnableWindow(owner, 0);
    }
    ShowWindow(hwnd, SW_SHOW);
    SetFocus(hwnd);

    let mut msg: MSG = zeroed();
    while !panel.done {
        let got = GetMessageW(&mut msg, null_mut(), 0, 0);
        if got <= 0 {
            // Appen er ved at lukke: giv beskeden videre til hovedløkken.
            if got == 0 {
                PostQuitMessage(msg.wParam as i32);
            }
            break;
        }
        TranslateMessage(&msg);
        DispatchMessageW(&msg);
    }

    // Ejeren aktiveres igen, før vinduet forsvinder; ellers giver Windows fokus til et andet program.
    if !owner.is_null() {
        EnableWindow(owner, 1);
        SetActiveWindow(owner);
    }
    SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
    DestroyWindow(hwnd);
}

/// De skrifttyper fra `FONTS` der findes på maskinen. Windows vælger stiltiende en
/// anden skrift, når den ønskede mangler, så vi spørger hvilken vi faktisk fik.
unsafe fn installed_fonts(owner: HWND) -> Vec<usize> {
    let hdc = GetDC(owner);
    let mut found = Vec::new();
    for (index, face) in FONTS.iter().enumerate() {
        let name = wide(face);
        let font: HFONT = CreateFontW(-16, 0, 0, 0, 400, 0, 0, 0, 1, 0, 0, 0, 0, name.as_ptr());
        let previous = SelectObject(hdc, font);
        let mut actual = [0u16; 64];
        let len = GetTextFaceW(hdc, actual.len() as i32, actual.as_mut_ptr()).max(0) as usize;
        SelectObject(hdc, previous);
        DeleteObject(font);
        if String::from_utf16_lossy(&actual[..len]).trim_end_matches('\0').eq_ignore_ascii_case(face) {
            found.push(index);
        }
    }
    ReleaseDC(owner, hdc);
    if found.is_empty() {
        found.push(0);
    }
    found
}

/// Fanernes tekstbredder, så den første fane kan flugte med rækkerne under den.
unsafe fn tab_widths(window: HWND, dpi: u32) -> [i32; PAGES] {
    let hdc = GetDC(window);
    let previous = SelectObject(hdc, ui::ui_font(dpi));
    let mut widths = [0; PAGES];
    for (page, width) in widths.iter_mut().enumerate() {
        let name: Vec<u16> = page_name(page).encode_utf16().collect();
        let mut measured: RECT = zeroed();
        DrawTextW(hdc, name.as_ptr(), name.len() as i32, &mut measured, DT_CALCRECT | DT_SINGLELINE | DT_NOPREFIX);
        *width = measured.right;
    }
    SelectObject(hdc, previous);
    ReleaseDC(window, hdc);
    widths
}

const PAD: i32 = 24;
const TABS: i32 = 44;
/// Luften på hver side af en fanes tekst; den hører med til det man kan klikke på.
const TAB_PAD: i32 = 12;
/// Rækkerne under Generelt er højere end genvejenes, så de to sider fylder lige meget.
const ROW_GENERAL: i32 = 44;
const ROW_SHORTCUT: i32 = 32;
/// Afstanden mellem rækkerne med flere valg og kontakterne under Generelt.
const GROUP_GAP: i32 = 20;
/// Højden af et felt eller en knap i en række.
const FIELD: i32 = 28;
const CONTROL: i32 = 230;
const BAND: i32 = 56;
const NOTE: i32 = 32;

fn rows_top(dpi: u32) -> i32 {
    scale(dpi, TABS + 12)
}

fn client_size(dpi: u32) -> (i32, i32) {
    let general = GENERAL as i32 * ROW_GENERAL + GROUP_GAP;
    let shortcuts = COMMANDS.len() as i32 * ROW_SHORTCUT;
    (scale(dpi, 560), rows_top(dpi) + scale(dpi, general.max(shortcuts)) + scale(dpi, NOTE + BAND))
}

fn row_rect(dpi: u32, page: usize, index: usize) -> RECT {
    let (width, _) = client_size(dpi);
    let height = scale(dpi, if page == 0 { ROW_GENERAL } else { ROW_SHORTCUT });
    let gap = if page == 0 && index >= STEPS { scale(dpi, GROUP_GAP) } else { 0 };
    let top = rows_top(dpi) + index as i32 * height + gap;
    RECT { left: scale(dpi, PAD), top, right: width - scale(dpi, PAD), bottom: top + height }
}

/// Feltet til højre i en række, centreret i rækkens højde.
fn control_rect(dpi: u32, row: &RECT) -> RECT {
    let top = (row.top + row.bottom - scale(dpi, FIELD)) / 2;
    RECT { left: row.right - scale(dpi, CONTROL), top, right: row.right, bottom: top + scale(dpi, FIELD) }
}

fn inset(r: &RECT, by: i32) -> RECT {
    RECT { left: r.left + by, top: r.top + by, right: r.right - by, bottom: r.bottom - by }
}

/// Alt hvad man kan klikke på, med det felt det fylder.
fn targets(panel: &Panel) -> Vec<(Target, RECT)> {
    let dpi = panel.dpi;
    let (width, height) = client_size(dpi);
    let mut out = Vec::new();
    let mut left = scale(dpi, PAD - TAB_PAD);
    for page in 0..PAGES {
        let right = left + panel.tabs[page] + 2 * scale(dpi, TAB_PAD);
        out.push((Target::Page(page), RECT { left, top: 0, right, bottom: scale(dpi, TABS) }));
        left = right;
    }
    if panel.page == 0 {
        for index in 0..GENERAL {
            let control = control_rect(dpi, &row_rect(dpi, 0, index));
            if index < STEPS {
                let arrow = scale(dpi, FIELD);
                out.push((Target::Step(index, false), RECT { right: control.left + arrow, ..control }));
                out.push((Target::Step(index, true), RECT { left: control.right - arrow, ..control }));
            } else {
                let middle = (control.top + control.bottom) / 2;
                let half = scale(dpi, 10);
                out.push((Target::Toggle(index), RECT { left: control.right - scale(dpi, 40), top: middle - half, right: control.right, bottom: middle + half }));
            }
        }
    } else {
        for index in 0..COMMANDS.len() {
            let control = control_rect(dpi, &row_rect(dpi, 1, index));
            let gap = scale(dpi, 8);
            let each = (scale(dpi, CONTROL) - gap) / 2;
            for slot in 0..2 {
                let left = control.left + slot as i32 * (each + gap);
                out.push((Target::Shortcut(index, slot), RECT { left, right: left + each, ..control }));
            }
        }
    }
    let band = height - scale(dpi, BAND);
    let button_h = scale(dpi, 30);
    let button_top = band + (scale(dpi, BAND) - button_h) / 2;
    let button = |left: i32, button_w: i32| RECT { left, top: button_top, right: left + button_w, bottom: button_top + button_h };
    out.push((Target::Reset, button(scale(dpi, PAD), scale(dpi, 170))));
    out.push((Target::Close, button(width - scale(dpi, PAD + 100), scale(dpi, 100))));
    out
}

fn target_at(panel: &Panel, x: i32, y: i32) -> Option<Target> {
    targets(panel).into_iter().find(|(_, r)| x >= r.left && x < r.right && y >= r.top && y < r.bottom).map(|(target, _)| target)
}

unsafe fn text(hdc: HDC, s: &str, r: &RECT, color: u32, format: u32) {
    // En tom tekst har ingen gyldig adresse at give DrawText.
    if s.is_empty() {
        return;
    }
    let wide: Vec<u16> = s.encode_utf16().collect();
    let mut r = *r;
    SetTextColor(hdc, color);
    DrawTextW(hdc, wide.as_ptr(), wide.len() as i32, &mut r, DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX | DT_END_ELLIPSIS | format);
}

/// Værdien der vises på en række med flere valg.
fn value_label(index: usize) -> String {
    let s = ui::settings();
    let words = lang::s();
    match index {
        0 => [words.follow_windows, words.light, words.dark][s.theme as usize % 3].to_string(),
        1 => s.font_face().to_string(),
        2 => format!("{} pt", s.size),
        3 => lang::fill(words.chars, &[&s.tab.to_string()]),
        _ => lang::choice_name(s.language).to_string(),
    }
}

unsafe fn paint(hwnd: HWND, panel: &Panel) {
    let mut ps: PAINTSTRUCT = zeroed();
    let hdc = BeginPaint(hwnd, &mut ps);
    let mut rc: RECT = zeroed();
    GetClientRect(hwnd, &mut rc);
    let theme = ui::theme();
    let dpi = panel.dpi;
    let s = ui::settings();

    let mem = CreateCompatibleDC(hdc);
    let bitmap = CreateCompatibleBitmap(hdc, rc.right.max(1), rc.bottom.max(1));
    let old_bitmap = SelectObject(mem, bitmap);
    let ui_font = ui::ui_font(dpi);
    let old_font = SelectObject(mem, ui_font);
    SetBkMode(mem, TRANSPARENT as _);

    // Hele vinduet har én baggrund; tynde linjer skiller faner og knapper fra rækkerne.
    fill(mem, &rc, theme.editor_bg);
    let line = scale(dpi, 1).max(1);
    let tabs_bottom = scale(dpi, TABS);
    fill(mem, &RECT { top: tabs_bottom - line, bottom: tabs_bottom, ..rc }, theme.sep);
    let band_top = rc.bottom - scale(dpi, BAND);
    fill(mem, &RECT { top: band_top, bottom: band_top + line, ..rc }, theme.sep);

    // Rækkernes tekster til venstre.
    let labels: Vec<&str> = if panel.page == 0 { general_labels().to_vec() } else { COMMANDS.iter().map(|c| c.label()).collect() };
    for (index, label) in labels.iter().enumerate() {
        let row = row_rect(dpi, panel.page, index);
        text(mem, label, &RECT { right: row.right - scale(dpi, CONTROL + 12), ..row }, theme.text, 0);
        if panel.page == 0 && index < STEPS {
            // Feltet med værdien, mellem de to pile.
            let control = control_rect(dpi, &row);
            field(mem, &control, dpi, theme.strip, theme.sep);
            text(mem, &value_label(index), &control, theme.text, DT_CENTER);
        }
    }
    if panel.page == 0 {
        // En linje mellem valgene og kontakterne.
        let middle = row_rect(dpi, 0, STEPS).top - scale(dpi, GROUP_GAP) / 2;
        fill(mem, &RECT { left: scale(dpi, PAD), top: middle, right: rc.right - scale(dpi, PAD), bottom: middle + line }, theme.sep);
    }

    for (target, r) in targets(panel) {
        let pressed = panel.pressed == Some(target);
        let hot = panel.hover == Some(target) || pressed;
        // Et felt man kan trykke på: lidt lysere under musen og mere, mens det holdes nede.
        let surface = if pressed { theme.close_hover } else if hot { theme.hover } else { theme.strip };
        match target {
            Target::Page(page) => {
                let active = page == panel.page;
                if active {
                    let pad = scale(dpi, TAB_PAD);
                    fill(mem, &RECT { left: r.left + pad, top: r.bottom - scale(dpi, 2), right: r.right - pad, bottom: r.bottom }, theme.accent);
                }
                text(mem, page_name(page), &r, if active || hot { theme.text } else { theme.dim }, DT_CENTER);
            }
            Target::Step(_, forward) => {
                if hot {
                    round(mem, &inset(&r, scale(dpi, 3)), scale(dpi, RADIUS - 1), if pressed { theme.close_hover } else { theme.hover }, theme.strip);
                }
                SelectObject(mem, ui::icon_font(dpi));
                let glyph = [if forward { GLYPH_RIGHT } else { GLYPH_LEFT }];
                let mut glyph_rect = r;
                SetTextColor(mem, if hot { theme.text } else { theme.dim });
                DrawTextW(mem, glyph.as_ptr(), 1, &mut glyph_rect, DT_SINGLELINE | DT_VCENTER | DT_CENTER | DT_NOPREFIX);
                SelectObject(mem, ui_font);
            }
            Target::Toggle(index) => {
                // En kontakt: farvet og med knoppen til højre, når den er slået til.
                let on = match index {
                    5 => s.wrap,
                    6 => s.syntax,
                    7 => s.line_numbers,
                    _ => s.whole_word,
                };
                let half = (r.bottom - r.top) / 2;
                let track = if on {
                    let track = if hot { mix(theme.accent, theme.editor_bg, 15) } else { theme.accent };
                    round(mem, &r, half, track, theme.editor_bg);
                    track
                } else {
                    let track = if hot { theme.hover } else { theme.editor_bg };
                    outlined(mem, &r, half, track, theme.dim, line, theme.editor_bg);
                    track
                };
                let gap = scale(dpi, 4);
                let knob = r.bottom - r.top - 2 * gap;
                let left = if on { r.right - gap - knob } else { r.left + gap };
                let knob_rect = RECT { left, top: r.top + gap, right: left + knob, bottom: r.top + gap + knob };
                round(mem, &knob_rect, knob / 2, if on { theme.on_accent } else { theme.dim }, track);
            }
            Target::Shortcut(command, slot) => {
                let waiting = panel.capture == Some((command, slot));
                let key = s.keys[command][slot];
                if waiting {
                    field(mem, &r, dpi, theme.accent, theme.accent);
                } else {
                    field(mem, &r, dpi, surface, theme.sep);
                }
                let (label, color) = if waiting {
                    (lang::s().press_keys.to_string(), theme.on_accent)
                } else if key == Key::NONE {
                    ("–".to_string(), theme.dim)
                } else {
                    (key.name(), theme.text)
                };
                text(mem, &label, &r, color, DT_CENTER);
            }
            Target::Reset | Target::Close => {
                let primary = target == Target::Close;
                if primary {
                    let back = mix(theme.accent, theme.editor_bg, if pressed { 30 } else if hot { 15 } else { 0 });
                    field(mem, &r, dpi, back, back);
                } else {
                    field(mem, &r, dpi, surface, theme.sep);
                }
                text(mem, if primary { lang::s().close } else { lang::s().reset }, &r, if primary { theme.on_accent } else { theme.text }, DT_CENTER);
            }
        }
    }

    // Linjen med hjælp eller besked lige over knapperne.
    let note = if !panel.note.is_empty() { panel.note.as_str() } else if panel.page == 1 { lang::s().hint } else { "" };
    let note_rect = RECT { left: scale(dpi, PAD), top: band_top - scale(dpi, NOTE), right: rc.right - scale(dpi, PAD), bottom: band_top };
    text(mem, note, &note_rect, theme.dim, 0);

    BitBlt(hdc, 0, 0, rc.right, rc.bottom, mem, 0, 0, SRCCOPY);
    SelectObject(mem, old_font);
    SelectObject(mem, old_bitmap);
    DeleteObject(bitmap);
    DeleteDC(mem);
    EndPaint(hwnd, &ps);
}

/// Udfører et klik. Ændringer slår igennem i appen og gemmes med det samme.
unsafe fn click(hwnd: HWND, panel: &mut Panel, target: Target) {
    let s = ui::settings();
    panel.note.clear();
    let capturing = panel.capture.take();
    let change = match target {
        Target::Page(page) => {
            panel.page = page;
            return;
        }
        Target::Step(0, forward) => {
            s.theme = (s.theme + if forward { 1 } else { 2 }) % 3;
            Change::Theme
        }
        Target::Step(1, forward) => {
            // Blad i de installerede skrifttyper.
            let count = panel.fonts.len();
            let at = panel.fonts.iter().position(|&f| f == s.font).unwrap_or(0);
            s.font = panel.fonts[(at + if forward { 1 } else { count - 1 }) % count];
            Change::Font
        }
        Target::Step(3, forward) => {
            let count = TABS_WIDTHS.len();
            let at = TABS_WIDTHS.iter().position(|&w| w == s.tab).unwrap_or(2);
            s.tab = TABS_WIDTHS[(at + if forward { 1 } else { count - 1 }) % count];
            Change::Font
        }
        Target::Step(4, forward) => {
            s.language = (s.language + if forward { 1 } else { lang::CHOICES - 1 }) % lang::CHOICES;
            Change::Language
        }
        Target::Step(_, forward) => {
            s.size = if forward { s.size + 1 } else { s.size.saturating_sub(1) }.clamp(*SIZES.start(), *SIZES.end());
            Change::Font
        }
        Target::Toggle(5) => {
            s.wrap = !s.wrap;
            Change::Wrap
        }
        Target::Toggle(7) => {
            s.line_numbers = !s.line_numbers;
            Change::Gutter
        }
        Target::Toggle(8) => {
            s.whole_word = !s.whole_word;
            Change::Find
        }
        Target::Toggle(_) => {
            s.syntax = !s.syntax;
            Change::Syntax
        }
        Target::Shortcut(command, slot) => {
            // Et klik mere på den samme fortryder.
            if capturing != Some((command, slot)) {
                panel.capture = Some((command, slot));
                panel.note = lang::s().press_new.to_string();
            }
            return;
        }
        Target::Reset => {
            *s = Settings::default();
            // Beskeden skal stå på det sprog, nulstillingen skifter til.
            lang::select(s.language);
            panel.note = lang::s().all_reset.to_string();
            Change::All
        }
        Target::Close => {
            panel.done = true;
            return;
        }
    };
    s.save();
    ui::apply_settings(change);
    // Et nyt tema eller sprog skal også nå dette vindues titellinje.
    ui::apply_frame_theme(hwnd);
    SetWindowTextW(hwnd, wide(lang::s().settings).as_ptr());
    panel.tabs = tab_widths(hwnd, panel.dpi);
}

/// Et tastetryk mens en genvej venter på sin nye kombination.
unsafe fn capture_key(panel: &mut Panel, vk: u16) {
    let Some((command, slot)) = panel.capture else { return };
    if [VK_CONTROL, VK_SHIFT, VK_MENU, VK_LWIN, VK_RWIN, 0xA0, 0xA1, 0xA2, 0xA3, 0xA4, 0xA5].contains(&vk) {
        // Kun en skiftetast indtil videre; vent på selve tasten.
        return;
    }
    let down = |key: u16| GetKeyState(key as i32) < 0;
    let key = Key::new(vk, down(VK_CONTROL), down(VK_MENU), down(VK_SHIFT));
    let plain = key.0 == vk as u32;
    let s = ui::settings();
    if plain && vk == VK_ESCAPE {
        panel.note.clear();
    } else if plain && (vk == VK_DELETE || vk == VK_BACK) {
        s.assign(command, slot, Key::NONE);
        panel.note = lang::fill(lang::s().shortcut_removed, &[COMMANDS[command].label()]);
    } else if let Some(problem) = key.problem() {
        // Bliv ved med at vente, så man kan prøve en anden.
        panel.note = problem.to_string();
        return;
    } else {
        panel.note = match s.assign(command, slot, key) {
            Some(other) => lang::fill(lang::s().shortcut_taken, &[&key.name(), COMMANDS[command].label(), other]),
            None => lang::fill(lang::s().shortcut_set, &[&key.name(), COMMANDS[command].label()]),
        };
    }
    panel.capture = None;
    s.save();
}

unsafe extern "system" fn panel_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    let state = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Panel;
    if state.is_null() {
        return DefWindowProcW(hwnd, msg, wp, lp);
    }
    let panel = &mut *state;
    match msg {
        WM_PAINT => {
            paint(hwnd, panel);
            0
        }
        WM_ERASEBKGND => 1,
        WM_MOUSEMOVE => {
            let over = target_at(panel, lo(lp), hi(lp));
            if over != panel.hover {
                panel.hover = over;
                InvalidateRect(hwnd, null(), 0);
            }
            0
        }
        WM_LBUTTONDOWN | WM_LBUTTONDBLCLK => {
            panel.pressed = target_at(panel, lo(lp), hi(lp));
            InvalidateRect(hwnd, null(), 0);
            0
        }
        WM_LBUTTONUP => {
            let released = target_at(panel, lo(lp), hi(lp));
            let pressed = panel.pressed.take();
            if let Some(target) = released.filter(|_| released == pressed) {
                click(hwnd, panel, target);
            }
            InvalidateRect(hwnd, null(), 0);
            0
        }
        WM_KEYDOWN | WM_SYSKEYDOWN => {
            let vk = wp as u16;
            if panel.capture.is_some() {
                capture_key(panel, vk);
            } else if msg == WM_SYSKEYDOWN {
                // Alt+F4 og de andre systemtaster virker som normalt.
                return DefWindowProcW(hwnd, msg, wp, lp);
            } else if vk == VK_ESCAPE || vk == VK_RETURN {
                panel.done = true;
            }
            InvalidateRect(hwnd, null(), 0);
            0
        }
        // Alt+bogstav må ikke bippe eller åbne vinduesmenuen, mens en genvej indtastes.
        WM_SYSCHAR if panel.capture.is_some() => 0,
        WM_CLOSE => {
            panel.done = true;
            0
        }
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_shortcuts_are_usable_and_unique() {
        let s = Settings::default();
        let mut seen = Vec::new();
        for (command, keys) in COMMANDS.iter().zip(&s.keys) {
            for key in keys.iter().filter(|k| **k != Key::NONE) {
                assert_eq!(key.problem(), None, "{}", command.label());
                assert!(!seen.contains(key), "{} deler genvej med en anden", command.label());
                assert_eq!(s.command_for(*key), Some(command.id));
                seen.push(*key);
            }
        }
        assert_eq!(s.command_for(Key::NONE), None);
    }

    #[test]
    fn unusable_combinations_are_rejected() {
        assert!(Key::new(b'S' as u16, false, false, false).problem().is_some());
        assert!(Key::new(b'S' as u16, false, false, true).problem().is_some());
        assert!(Key::new(b'Q' as u16, true, true, false).problem().is_some());
        assert!(Key::new(b'C' as u16, true, false, false).problem().is_some());
        assert!(Key::new(b'3' as u16, true, false, false).problem().is_some());
        assert!(Key::new(VK_F4, false, true, false).problem().is_some());
        // Disse er i orden.
        assert_eq!(Key::new(b'C' as u16, true, false, true).problem(), None);
        assert_eq!(Key::new(VK_F1 + 4, false, false, false).problem(), None);
        assert_eq!(Key::new(b'Q' as u16, false, true, false).problem(), None);
    }

    #[test]
    fn assigning_a_used_shortcut_takes_it_from_the_other_command() {
        let mut s = Settings::default();
        let save = Key::ctrl(b'S' as u16);
        // Giv "Luk fane" (nr. 4) genvejen fra "Gem" (nr. 0).
        assert_eq!(s.assign(4, 0, save), Some("Gem"));
        assert_eq!(s.keys[0][0], Key::NONE);
        assert_eq!(s.command_for(save), Some(CMD_CLOSE));
        // At flytte en genvej mellem kommandoens egne to pladser tager den ikke fra nogen.
        assert_eq!(s.assign(4, 1, save), None);
        assert_eq!(s.keys[4], [Key::NONE, save]);
        // Og at fjerne en genvej rører ikke de andre.
        assert_eq!(s.assign(4, 1, Key::NONE), None);
        assert_eq!(s.command_for(Key::ctrl(b'N' as u16)), Some(CMD_NEW));
    }

    #[test]
    fn shortcut_names_read_naturally() {
        assert_eq!(Key::new(b'S' as u16, true, false, true).name(), "Ctrl+Shift+S");
        assert_eq!(Key::ctrl(VK_TAB).name(), "Ctrl+Tab");
        assert_eq!(Key::ctrl(VK_F4).name(), "Ctrl+F4");
        assert_eq!(Key::new(VK_F1 + 11, false, false, false).name(), "F12");
        assert_eq!(Key::ctrl(VK_NEXT).name(), "Ctrl+PgDn");
        assert_eq!(Key::NONE.name(), "");
        assert_eq!(Settings::default().shortcut(CMD_NEW), "Ctrl+N");
    }
}
