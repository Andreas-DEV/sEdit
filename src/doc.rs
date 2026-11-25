//! Et dokument: filens tegnsæt og linjeskift samt RichEdit-feltet der viser den.

use std::io;
use std::mem::{size_of, zeroed};
use std::path::{Path, PathBuf};
use std::ptr::{null, null_mut};
use std::sync::atomic::{AtomicI32, AtomicIsize, Ordering};
use std::time::SystemTime;

use windows_sys::w;
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, SIZE, WPARAM};
use windows_sys::Win32::Globalization::{MultiByteToWideChar, WideCharToMultiByte, CP_ACP};
use windows_sys::Win32::Graphics::Gdi::{
    GetDC, GetTextExtentPoint32W, RedrawWindow, ReleaseDC, SelectObject, HFONT, RDW_ALLCHILDREN, RDW_ERASE,
    RDW_FRAME, RDW_INVALIDATE,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Controls::SetWindowTheme;
use windows_sys::Win32::UI::HiDpi::GetDpiForWindow;
use windows_sys::Win32::UI::Shell::DragAcceptFiles;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CallWindowProcW, CreatePopupMenu, CreateWindowExW, DestroyMenu, GetAncestor,
    GetCursorPos, IsWindowVisible, PostMessageW, SendMessageW, SetWindowLongPtrW, TrackPopupMenu,
    GA_ROOT, GWLP_WNDPROC, MF_CHECKED, MF_GRAYED, MF_SEPARATOR, MF_STRING, TPM_RETURNCMD,
    TPM_RIGHTBUTTON, WM_CLEAR, WM_COMMAND, WM_CONTEXTMENU, WM_COPY, WM_CUT, WM_DROPFILES,
    SystemParametersInfoW, WM_DESTROY, WM_GETFONT, WM_MOUSEWHEEL, WM_PAINT, WM_PASTE, WM_SETFONT, WM_SETREDRAW, WM_VSCROLL,
    WM_SETTEXT, WM_TIMER, WM_UNDO, WS_CHILD, WS_HSCROLL, WS_VSCROLL,
};

use crate::highlight;
use crate::theme::Theme;
use crate::util::{lo, hi};

// RichEdit-beskeder og -stilarter (richedit.h / winuser.h).
const WM_USER: u32 = 0x0400;
const EM_SETSEL: u32 = 0x00B1;
const EM_GETMODIFY: u32 = 0x00B8;
const EM_SETMODIFY: u32 = 0x00B9;
const EM_REPLACESEL: u32 = 0x00C2;
const EM_CANUNDO: u32 = 0x00C6;
const EM_EMPTYUNDOBUFFER: u32 = 0x00CD;
const EM_SETMARGINS: u32 = 0x00D3;
const EM_CANPASTE: u32 = WM_USER + 50;
const EM_EXGETSEL: u32 = WM_USER + 52;
const EM_EXLIMITTEXT: u32 = WM_USER + 53;
const EM_SETLANGOPTIONS: u32 = WM_USER + 120;
const EM_EXSETSEL: u32 = WM_USER + 55;
const EM_SETBKGNDCOLOR: u32 = WM_USER + 67;
const EM_SETCHARFORMAT: u32 = WM_USER + 68;
const EM_SETTARGETDEVICE: u32 = WM_USER + 72;
const EM_SETTEXTMODE: u32 = WM_USER + 89;
const EM_GETTEXTEX: u32 = WM_USER + 94;
const EM_GETTEXTLENGTHEX: u32 = WM_USER + 95;
const EM_GETSCROLLPOS: u32 = WM_USER + 221;
const EM_SETSCROLLPOS: u32 = WM_USER + 222;

const ES_MULTILINE: u32 = 0x0004;
const ES_NOOLEDRAGDROP: u32 = 0x0008;
const ES_AUTOVSCROLL: u32 = 0x0040;
const ES_AUTOHSCROLL: u32 = 0x0080;
const ES_NOHIDESEL: u32 = 0x0100;
const ES_WANTRETURN: u32 = 0x1000;

const TM_RICHTEXT: usize = 2;
const TM_MULTILEVELUNDO: usize = 8;
const TM_MULTICODEPAGE: usize = 32;
const GTL_PRECISE: u32 = 2;
const GTL_NUMCHARS: u32 = 8;
const CFM_COLOR: u32 = 0x4000_0000;
const SCF_DEFAULT: usize = 0;
const SCF_ALL: usize = 4;
const CF_UNICODETEXT: usize = 13;
const EM_LINESCROLL: u32 = 0x00B6;
const MK_SHIFT: usize = 0x0004;
const MK_CONTROL: usize = 0x0008;
const SPI_GETWHEELSCROLLLINES: u32 = 0x0068;
const WHEEL_DELTA: i32 = 120;
const SB_PAGEUP: usize = 2;
const SB_PAGEDOWN: usize = 3;
const EC_LEFTMARGIN: usize = 1;
const EC_RIGHTMARGIN: usize = 2;
const CP_UTF16: u32 = 1200;

// RichEdits strukturer findes ikke i windows-sys. richedit.h pakker dem til 4 bytes,
// så pointerne i GETTEXTEX ligger på offset 12 og ikke 16.
#[repr(C, packed(4))]
#[allow(non_snake_case)]
struct CHARFORMATW {
    cbSize: u32,
    dwMask: u32,
    dwEffects: u32,
    yHeight: i32,
    yOffset: i32,
    crTextColor: u32,
    bCharSet: u8,
    bPitchAndFamily: u8,
    szFaceName: [u16; 32],
}

#[repr(C)]
#[allow(non_snake_case)]
struct CHARRANGE {
    cpMin: i32,
    cpMax: i32,
}

#[repr(C, packed(4))]
#[allow(non_snake_case)]
struct GETTEXTEX {
    cb: u32,
    flags: u32,
    codepage: u32,
    lpDefaultChar: *const u8,
    lpUsedDefChar: *mut i32,
}

#[repr(C)]
struct GETTEXTLENGTHEX {
    flags: u32,
    codepage: u32,
}

const CR: u16 = 13;
const LF: u16 = 10;
const VT: u16 = 11;

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Enc {
    Utf8,
    Utf8Bom,
    Utf16Le,
    Utf16Be,
    Ansi,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Eol {
    Crlf,
    Lf,
    Cr,
}

/// Ændringstid og størrelse: nok til at opdage at filen er skrevet af andre.
pub type Stamp = (SystemTime, u64);

pub fn stat(path: &Path) -> Option<Stamp> {
    let meta = std::fs::metadata(path).ok()?;
    Some((meta.modified().ok()?, meta.len()))
}

/// Fingeraftryk af en tekst: længde plus hash. Bruges til at afgøre, om det der står i
/// tekstfeltet er det samme som det der ligger på disken, uden at gemme en kopi af teksten.
/// Det skrives også i sessionsfilen, så hashen skal være stabil på tværs af versioner.
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct Sig {
    pub len: usize,
    pub hash: u64,
}

pub fn signature(text: &[u16]) -> Sig {
    // FNV-1a over tekstens bytes.
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for unit in text {
        for byte in unit.to_le_bytes() {
            hash ^= byte as u64;
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    Sig { len: text.len(), hash }
}

/// Over denne længde sammenlignes teksten ikke ved hvert tastetryk, kun når det gælder
/// (ved lukning og når filen ændres udefra). Ellers ville store filer føles træge.
const LIVE_COMPARE_LIMIT: usize = 2_000_000;

pub struct Doc {
    pub id: u32,
    pub path: Option<PathBuf>,
    /// Stien i små bogstaver, til at genkende en fil der allerede er åben.
    pub key: String,
    pub name: String,
    /// Teksten på fanen (kan indeholde mappenavn, hvis to faner hedder det samme).
    pub label: Vec<u16>,
    /// Fanetekstens bredde i pixels; 0 = ikke målt endnu.
    pub label_w: i32,
    pub edit: HWND,
    pub dpi: u32,
    pub enc: Enc,
    pub eol: Eol,
    pub dirty: bool,
    pub stamp: Option<Stamp>,
    /// Fingeraftryk af teksten, som den sidst blev læst fra eller skrevet til disken.
    pub saved: Sig,
}

impl Doc {
    /// Afgør om teksten afviger fra filen på disken, og opdaterer `dirty`.
    ///
    /// Tekstfeltets eget ændringsflag siger kun "der er rettet noget". Fortryder man
    /// tilbage til det filen indeholder, står flaget stadig, så her sammenlignes selve
    /// teksten. `thorough` sammenligner også meget store filer.
    pub unsafe fn refresh_dirty(&mut self, thorough: bool) -> bool {
        self.dirty = if !is_modified(self.edit) {
            false
        } else {
            let len = text_len(self.edit);
            if len != self.saved.len || (!thorough && len > LIVE_COMPARE_LIMIT) {
                true
            } else if signature(&get_text(self.edit)) == self.saved {
                // Tilbage ved filens indhold: nulstil flaget, så næste tjek er gratis.
                set_modified(self.edit, false);
                false
            } else {
                true
            }
        };
        self.dirty
    }

    /// Læser filen ind i tekstfeltet og beholder markør og rulleposition.
    pub unsafe fn load(&mut self) -> io::Result<()> {
        let Some(path) = self.path.as_ref() else { return Ok(()) };
        // Stemplet tages før læsningen: skrives filen imens, opdages det næste gang.
        let stamp = stat(path);
        let bytes = std::fs::read(path)?;
        let (mut text, enc, eol) = decode(&bytes);
        replace_text(self.edit, &mut text);
        self.enc = enc;
        self.eol = eol;
        self.stamp = stamp;
        self.dirty = false;
        // Fingeraftrykket tages af det tekstfeltet faktisk rummer, så det altid kan
        // sammenlignes med senere udtræk fra samme felt.
        self.saved = signature(&get_text(self.edit));
        Ok(())
    }

    pub unsafe fn save(&mut self) -> io::Result<()> {
        let Some(path) = self.path.as_ref() else { return Ok(()) };
        let text = get_text(self.edit);
        std::fs::write(path, encode(&text, self.enc, self.eol))?;
        // Nyt stempel, så vores egen skrivning ikke udløser en genindlæsning.
        self.stamp = stat(path);
        self.saved = signature(&text);
        set_modified(self.edit, false);
        self.dirty = false;
        Ok(())
    }
}

// ---------- tegnsæt og linjeskift ----------

/// Bytes -> UTF-16 med CR som eneste linjeskift (RichEdits interne form).
pub fn decode(bytes: &[u8]) -> (Vec<u16>, Enc, Eol) {
    let (raw, enc): (Vec<u16>, Enc) = if let Some(rest) = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        (String::from_utf8_lossy(rest).encode_utf16().collect(), Enc::Utf8Bom)
    } else if let Some(rest) = bytes.strip_prefix(&[0xFF, 0xFE]) {
        (rest.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect(), Enc::Utf16Le)
    } else if let Some(rest) = bytes.strip_prefix(&[0xFE, 0xFF]) {
        (rest.chunks_exact(2).map(|c| u16::from_be_bytes([c[0], c[1]])).collect(), Enc::Utf16Be)
    } else {
        match std::str::from_utf8(bytes) {
            Ok(s) => (s.encode_utf16().collect(), Enc::Utf8),
            Err(_) => (ansi_to_wide(bytes), Enc::Ansi),
        }
    };

    let mut text = Vec::with_capacity(raw.len() + 1);
    let (mut crlf, mut lf, mut cr) = (0usize, 0usize, 0usize);
    let mut i = 0;
    while i < raw.len() {
        match raw[i] {
            CR => {
                if raw.get(i + 1) == Some(&LF) {
                    crlf += 1;
                    i += 1;
                } else {
                    cr += 1;
                }
                text.push(CR);
            }
            LF => {
                lf += 1;
                text.push(CR);
            }
            c => text.push(c),
        }
        i += 1;
    }
    let eol = if lf > crlf && lf >= cr {
        Eol::Lf
    } else if cr > crlf && cr > lf {
        Eol::Cr
    } else {
        Eol::Crlf
    };
    (text, enc, eol)
}

/// UTF-16 med CR-linjeskift -> bytes i filens oprindelige tegnsæt og linjeskift.
pub fn encode(text: &[u16], enc: Enc, eol: Eol) -> Vec<u8> {
    let mut units = Vec::with_capacity(text.len() + text.len() / 16 + 2);
    for &c in text {
        // VT er RichEdits bløde linjeskift (Shift+Enter).
        if c == CR || c == VT {
            match eol {
                Eol::Crlf => units.extend_from_slice(&[CR, LF]),
                Eol::Lf => units.push(LF),
                Eol::Cr => units.push(CR),
            }
        } else {
            units.push(c);
        }
    }
    match enc {
        Enc::Utf8 => String::from_utf16_lossy(&units).into_bytes(),
        Enc::Utf8Bom => {
            let mut out = vec![0xEF, 0xBB, 0xBF];
            out.extend_from_slice(String::from_utf16_lossy(&units).as_bytes());
            out
        }
        Enc::Utf16Le => {
            let mut out = vec![0xFF, 0xFE];
            out.extend(units.iter().flat_map(|u| u.to_le_bytes()));
            out
        }
        Enc::Utf16Be => {
            let mut out = vec![0xFE, 0xFF];
            out.extend(units.iter().flat_map(|u| u.to_be_bytes()));
            out
        }
        Enc::Ansi => wide_to_ansi(&units),
    }
}

// ---------- luft efter kommentarer ----------

fn trimmed(line: &[u16]) -> &[u16] {
    let blank = |c: &u16| *c == b' ' as u16 || *c == b'\t' as u16;
    let start = line.iter().position(|c| !blank(c)).unwrap_or(line.len());
    let end = line.iter().rposition(|c| !blank(c)).map_or(start, |i| i + 1);
    &line[start..end]
}

/// En linje der starter med `#`, `;` eller `//`.
fn is_comment(line: &[u16]) -> bool {
    match trimmed(line) {
        [first, ..] if *first == b'#' as u16 || *first == b';' as u16 => true,
        [a, b, ..] => *a == b'/' as u16 && *b == b'/' as u16,
        _ => false,
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Line {
    Comment,
    Text,
    Blank,
}

/// Positionerne hvor der skal indsættes en tom linje. Reglen ser på hver blok af
/// sammenhængende kommentarlinjer:
///
/// - Står der tekst lige under blokken, kommer den tomme linje mellem blokken og teksten.
/// - Hører blokken ikke til nogen tekst under sig (det næste der ikke er en tom linje er
///   en kommentar eller filens slutning), men står der tekst lige over den, kommer den
///   tomme linje over blokken, så den står for sig selv.
///
/// "Tekst under sig" ser forbi tomme linjer. Ellers ville en blok, der lige har fået sin
/// tomme linje, ligne en blok uden tekst under sig, og næste tryk ville sætte endnu en
/// linje ind over den. Med den regel ændrer et andet tryk intet.
///
/// `text` har CR som linjeskift; hver position er starten af en linje.
pub fn blank_line_offsets(text: &[u16]) -> Vec<usize> {
    let mut lines: Vec<(usize, Line)> = Vec::new();
    let mut start = 0;
    for line in text.split(|&c| c == CR) {
        let kind = if is_comment(line) {
            Line::Comment
        } else if trimmed(line).is_empty() {
            Line::Blank
        } else {
            Line::Text
        };
        lines.push((start, kind));
        start += line.len() + 1;
    }
    let kind_at = |index: usize| lines.get(index).map(|&(_, kind)| kind);

    let mut offsets = Vec::new();
    let mut first = 0;
    while first < lines.len() {
        if lines[first].1 != Line::Comment {
            first += 1;
            continue;
        }
        let mut after = first;
        while kind_at(after) == Some(Line::Comment) {
            after += 1;
        }
        let mut below = after;
        while kind_at(below) == Some(Line::Blank) {
            below += 1;
        }
        if kind_at(after) == Some(Line::Text) {
            offsets.push(lines[after].0);
        } else if kind_at(below) != Some(Line::Text) && first > 0 && kind_at(first - 1) == Some(Line::Text) {
            offsets.push(lines[first].0);
        }
        first = after;
    }
    offsets
}

/// Indsætter de tomme linjer som ét trin, der kan fortrydes med Ctrl+Z.
/// Returnerer false, hvis der ikke var noget at indsætte.
pub unsafe fn space_out_comments(edit: HWND) -> bool {
    let text = get_text(edit);
    let offsets = blank_line_offsets(&text);
    if offsets.is_empty() {
        return false;
    }
    let mut spaced = Vec::with_capacity(text.len() + offsets.len() + 1);
    let mut pending = offsets.iter().copied().peekable();
    for (i, &c) in text.iter().enumerate() {
        if pending.next_if_eq(&i).is_some() {
            spaced.push(CR);
        }
        spaced.push(c);
    }

    // Markøren følger med den tekst den stod i, selv om der er kommet linjer til ovenfor.
    let shifted = |position: i32| position + offsets.iter().filter(|&&o| o as i32 <= position).count() as i32;
    let (start, end) = selection(edit);
    replace_undoable(edit, &spaced, (shifted(start), shifted(end)));
    true
}

pub unsafe fn selection(edit: HWND) -> (i32, i32) {
    let mut range = CHARRANGE { cpMin: 0, cpMax: 0 };
    SendMessageW(edit, EM_EXGETSEL, 0, &mut range as *mut CHARRANGE as LPARAM);
    (range.cpMin, range.cpMax)
}

pub unsafe fn replace_selection(edit: HWND, select: (i32, i32)) {
    let range = CHARRANGE { cpMin: select.0, cpMax: select.1 };
    SendMessageW(edit, EM_EXSETSEL, 0, &range as *const CHARRANGE as LPARAM);
}

/// Udskifter hele teksten som ét trin, der kan fortrydes med Ctrl+Z, og sætter
/// markeringen bagefter. Rullepositionen bevares.
pub unsafe fn replace_undoable(edit: HWND, text: &[u16], select: (i32, i32)) {
    let mut terminated = Vec::with_capacity(text.len() + 1);
    terminated.extend_from_slice(text);
    terminated.push(0);
    let mut scroll = POINT { x: 0, y: 0 };
    SendMessageW(edit, EM_GETSCROLLPOS, 0, &mut scroll as *mut POINT as LPARAM);
    let everything = CHARRANGE { cpMin: 0, cpMax: -1 };
    let after = CHARRANGE { cpMin: select.0, cpMax: select.1 };

    let visible = IsWindowVisible(edit) != 0;
    if visible {
        SendMessageW(edit, WM_SETREDRAW, 0, 0);
    }
    SendMessageW(edit, EM_EXSETSEL, 0, &everything as *const CHARRANGE as LPARAM);
    SendMessageW(edit, EM_REPLACESEL, 1, terminated.as_ptr() as LPARAM);
    SendMessageW(edit, EM_EXSETSEL, 0, &after as *const CHARRANGE as LPARAM);
    SendMessageW(edit, EM_SETSCROLLPOS, 0, &scroll as *const POINT as LPARAM);
    if visible {
        SendMessageW(edit, WM_SETREDRAW, 1, 0);
        RedrawWindow(edit, null(), null_mut(), RDW_INVALIDATE | RDW_ERASE | RDW_FRAME | RDW_ALLCHILDREN);
    }
}

fn ansi_to_wide(bytes: &[u8]) -> Vec<u16> {
    if bytes.is_empty() {
        return Vec::new();
    }
    unsafe {
        let len = bytes.len() as i32;
        let n = MultiByteToWideChar(CP_ACP, 0, bytes.as_ptr(), len, null_mut(), 0);
        let mut out = vec![0u16; n.max(0) as usize];
        MultiByteToWideChar(CP_ACP, 0, bytes.as_ptr(), len, out.as_mut_ptr(), n);
        out
    }
}

fn wide_to_ansi(units: &[u16]) -> Vec<u8> {
    if units.is_empty() {
        return Vec::new();
    }
    unsafe {
        let len = units.len() as i32;
        let n = WideCharToMultiByte(CP_ACP, 0, units.as_ptr(), len, null_mut(), 0, null(), null_mut());
        let mut out = vec![0u8; n.max(0) as usize];
        WideCharToMultiByte(CP_ACP, 0, units.as_ptr(), len, out.as_mut_ptr(), n, null(), null_mut());
        out
    }
}

// ---------- tekstfeltet ----------

static ORIGINAL_EDIT_PROC: AtomicIsize = AtomicIsize::new(0);

/// Slår ombrydning af lange linjer til eller fra. Det ændrer kun visningen, ikke teksten.
pub unsafe fn set_wrap(edit: HWND, wrap: bool) {
    // Uden målenhed betyder linjebredde 0 "ombryd ved feltets kant" og 1 "ombryd aldrig".
    SendMessageW(edit, EM_SETTARGETDEVICE, 0, if wrap { 0 } else { 1 });
}

pub unsafe fn create_edit(parent: HWND, font: HFONT, theme: &Theme, wrap: bool) -> HWND {
    let edit = CreateWindowExW(
        0,
        w!("RICHEDIT50W"),
        null(),
        WS_CHILD
            | WS_VSCROLL
            | WS_HSCROLL
            | ES_MULTILINE
            | ES_AUTOVSCROLL
            | ES_AUTOHSCROLL
            | ES_NOHIDESEL
            | ES_WANTRETURN
            | ES_NOOLEDRAGDROP,
        0,
        0,
        0,
        0,
        parent,
        null_mut(),
        GetModuleHandleW(null()),
        null(),
    );
    // Rig tekst, fordi kode skal kunne farves. Feltet opfører sig stadig som ren tekst:
    // indsæt giver kun tekst (se `edit_proc`), og formateringsgenvejene sluges i `ui`.
    SendMessageW(edit, EM_SETTEXTMODE, TM_RICHTEXT | TM_MULTILEVELUNDO | TM_MULTICODEPAGE, 0);
    SendMessageW(edit, EM_EXLIMITTEXT, 0, 0x7FFF_FFFE);
    // RichEdit må ikke selv skifte skrifttype efter tastatursprog eller tegn; så ville
    // det man skriver forrest i teksten få en anden skrift end resten.
    SendMessageW(edit, EM_SETLANGOPTIONS, 0, 0);
    SendMessageW(edit, EM_SETMARGINS, EC_LEFTMARGIN | EC_RIGHTMARGIN, (6 << 16) | 6);
    style_edit(edit, font, theme);
    set_wrap(edit, wrap);
    SendMessageW(edit, EM_EMPTYUNDOBUFFER, 0, 0);
    set_modified(edit, false);

    // ES_NOOLEDRAGDROP slår RichEdits egen OLE-drop fra, så filer der slippes
    // på teksten når frem som WM_DROPFILES og åbnes som faner.
    DragAcceptFiles(edit, 1);
    let previous = SetWindowLongPtrW(edit, GWLP_WNDPROC, edit_proc as *const () as isize);
    ORIGINAL_EDIT_PROC.store(previous, Ordering::Relaxed);
    edit
}

/// Skrifttype og farver. Kaldes ved oprettelse samt når DPI eller tema skifter.
pub unsafe fn style_edit(edit: HWND, font: HFONT, theme: &Theme) {
    let modified = is_modified(edit);
    SendMessageW(edit, WM_SETFONT, font as WPARAM, 0);
    SendMessageW(edit, EM_SETBKGNDCOLOR, 0, theme.editor_bg as LPARAM);
    let mut format: CHARFORMATW = zeroed();
    format.cbSize = size_of::<CHARFORMATW>() as u32;
    format.dwMask = CFM_COLOR;
    format.crTextColor = theme.editor_fg;
    SendMessageW(edit, EM_SETCHARFORMAT, SCF_DEFAULT, &format as *const CHARFORMATW as LPARAM);
    SendMessageW(edit, EM_SETCHARFORMAT, SCF_ALL, &format as *const CHARFORMATW as LPARAM);
    let scrollbars = if theme.dark { w!("DarkMode_Explorer") } else { w!("Explorer") };
    SetWindowTheme(edit, scrollbars, null());

    apply_tab_width(edit, font);

    set_modified(edit, modified);
    // Al tekst har lige fået samme farve, så koden skal farves igen.
    highlight::invalidate(edit);
}

/// Et tekstfelt på én linje til søgelinjen. Det er ren tekst, så indsat tekst mister sin
/// formatering, og det fortæller sit forældervindue, når indholdet ændres (EN_CHANGE).
pub unsafe fn create_input(parent: HWND, font: HFONT, background: u32, foreground: u32) -> HWND {
    const TM_PLAINTEXT: usize = 1;
    const EM_SETEVENTMASK: u32 = WM_USER + 69;
    const ENM_CHANGE: LPARAM = 1;
    const WS_VISIBLE: u32 = 0x1000_0000;
    let input = CreateWindowExW(
        0,
        w!("RICHEDIT50W"),
        null(),
        WS_CHILD | WS_VISIBLE | ES_AUTOHSCROLL,
        0,
        0,
        0,
        0,
        parent,
        null_mut(),
        GetModuleHandleW(null()),
        null(),
    );
    SendMessageW(input, EM_SETTEXTMODE, TM_PLAINTEXT | TM_MULTILEVELUNDO | TM_MULTICODEPAGE, 0);
    SendMessageW(input, EM_SETLANGOPTIONS, 0, 0);
    SendMessageW(input, EM_SETMARGINS, EC_LEFTMARGIN | EC_RIGHTMARGIN, 0);
    style_input(input, font, background, foreground);
    SendMessageW(input, EM_SETEVENTMASK, 0, ENM_CHANGE);
    input
}

/// Skrift og farver til et felt fra `create_input`.
pub unsafe fn style_input(input: HWND, font: HFONT, background: u32, foreground: u32) {
    SendMessageW(input, WM_SETFONT, font as WPARAM, 0);
    SendMessageW(input, EM_SETBKGNDCOLOR, 0, background as LPARAM);
    let mut format: CHARFORMATW = zeroed();
    format.cbSize = size_of::<CHARFORMATW>() as u32;
    format.dwMask = CFM_COLOR;
    format.crTextColor = foreground;
    SendMessageW(input, EM_SETCHARFORMAT, SCF_DEFAULT, &format as *const CHARFORMATW as LPARAM);
    SendMessageW(input, EM_SETCHARFORMAT, SCF_ALL, &format as *const CHARFORMATW as LPARAM);
}

/// Sætter tabulatorbredden: et antal tegn i feltets skrift, omregnet til punkter.
/// Tekstfeltet glemmer bredden, når hele teksten skiftes ud, så det gøres også der.
unsafe fn apply_tab_width(edit: HWND, font: HFONT) {
    let hdc = GetDC(edit);
    let previous = SelectObject(hdc, font);
    let mut zero = SIZE { cx: 0, cy: 0 };
    GetTextExtentPoint32W(hdc, w!("0"), 1, &mut zero);
    SelectObject(hdc, previous);
    ReleaseDC(edit, hdc);
    let dpi = GetDpiForWindow(edit).max(1) as f32;
    highlight::set_tab_width(edit, zero.cx as f32 * 72.0 / dpi * crate::ui::settings().tab as f32);
}

pub unsafe fn is_modified(edit: HWND) -> bool {
    SendMessageW(edit, EM_GETMODIFY, 0, 0) != 0
}

pub unsafe fn set_modified(edit: HWND, modified: bool) {
    SendMessageW(edit, EM_SETMODIFY, modified as WPARAM, 0);
}

pub unsafe fn text_len(edit: HWND) -> usize {
    let query = GETTEXTLENGTHEX { flags: GTL_NUMCHARS | GTL_PRECISE, codepage: CP_UTF16 };
    SendMessageW(edit, EM_GETTEXTLENGTHEX, &query as *const GETTEXTLENGTHEX as WPARAM, 0).max(0) as usize
}

/// Feltets tekst som UTF-16 med CR som linjeskift.
pub unsafe fn get_text(edit: HWND) -> Vec<u16> {
    let len = text_len(edit);
    let mut buf = vec![0u16; len + 1];
    let request = GETTEXTEX {
        cb: (buf.len() * 2) as u32,
        flags: 0,
        codepage: CP_UTF16,
        lpDefaultChar: null(),
        lpUsedDefChar: null_mut(),
    };
    let got = SendMessageW(
        edit,
        EM_GETTEXTEX,
        &request as *const GETTEXTEX as WPARAM,
        buf.as_mut_ptr() as LPARAM,
    );
    buf.truncate(got.max(0) as usize);
    buf
}

/// Udskifter hele teksten uden at flytte markør eller rulleposition.
unsafe fn replace_text(edit: HWND, text: &mut Vec<u16>) {
    let visible = IsWindowVisible(edit) != 0;
    let mut selection = CHARRANGE { cpMin: 0, cpMax: 0 };
    let mut scroll = POINT { x: 0, y: 0 };
    SendMessageW(edit, EM_EXGETSEL, 0, &mut selection as *mut CHARRANGE as LPARAM);
    SendMessageW(edit, EM_GETSCROLLPOS, 0, &mut scroll as *mut POINT as LPARAM);
    if visible {
        SendMessageW(edit, WM_SETREDRAW, 0, 0);
    }
    text.push(0);
    SendMessageW(edit, WM_SETTEXT, 0, text.as_ptr() as LPARAM);
    text.pop();
    SendMessageW(edit, EM_EXSETSEL, 0, &selection as *const CHARRANGE as LPARAM);
    SendMessageW(edit, EM_SETSCROLLPOS, 0, &scroll as *const POINT as LPARAM);
    apply_tab_width(edit, SendMessageW(edit, WM_GETFONT, 0, 0) as HFONT);
    set_modified(edit, false);
    if visible {
        SendMessageW(edit, WM_SETREDRAW, 1, 0);
        RedrawWindow(edit, null(), null_mut(), RDW_INVALIDATE | RDW_ERASE | RDW_FRAME | RDW_ALLCHILDREN);
    }
}

unsafe extern "system" fn edit_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    let original: windows_sys::Win32::UI::WindowsAndMessaging::WNDPROC =
        std::mem::transmute(ORIGINAL_EDIT_PROC.load(Ordering::Relaxed));
    match msg {
        WM_DROPFILES => {
            crate::ui::on_drop_files(GetAncestor(hwnd, GA_ROOT), wp as _);
            return 0;
        }
        WM_CONTEXTMENU => {
            context_menu(hwnd, lp);
            return 0;
        }
        // Indsæt giver altid ren tekst, aldrig skrifttyper, farver eller billeder.
        WM_PASTE => {
            let result = CallWindowProcW(original, hwnd, highlight::EM_PASTESPECIAL, CF_UNICODETEXT, 0);
            highlight::text_changed(hwnd);
            return result;
        }
        WM_PAINT => highlight::before_paint(hwnd),
        // Musehjulet ruller med det samme. RichEdits egen bløde rulning sætter hakkene
        // i kø og er længe om at indhente dem. Ctrl (zoom) og Shift går videre uændret.
        WM_MOUSEWHEEL if wp & (MK_CONTROL | MK_SHIFT) == 0 => {
            wheel(hwnd, (wp >> 16) as i16 as i32);
            return 0;
        }
        WM_TIMER if wp == highlight::TIMER => {
            highlight::idle(hwnd);
            return 0;
        }
        WM_DESTROY => highlight::destroy(hwnd),
        _ => {}
    }
    let result = CallWindowProcW(original, hwnd, msg, wp, lp);
    if highlight::may_change_text(msg) {
        highlight::text_changed(hwnd);
    }
    if msg == WM_PAINT {
        // Teksten er tegnet (rullet, rettet eller ombrudt): linjenumrene skal følge med.
        crate::ui::edit_painted(hwnd);
    }
    result
}

/// Ruller teksten så mange linjer, som Windows er sat til pr. hak på musehjulet.
/// Pegefelter sender mange små bevægelser; resten gemmes, til der er nok til en linje.
unsafe fn wheel(edit: HWND, delta: i32) {
    static REST: AtomicI32 = AtomicI32::new(0);
    let mut lines: u32 = 3;
    SystemParametersInfoW(SPI_GETWHEELSCROLLLINES, 0, &mut lines as *mut u32 as *mut _, 0);
    if lines == u32::MAX {
        // Windows er sat til at rulle en hel side pr. hak.
        let total = REST.load(Ordering::Relaxed) + delta;
        REST.store(total % WHEEL_DELTA, Ordering::Relaxed);
        for _ in 0..(total / WHEEL_DELTA).abs() {
            SendMessageW(edit, WM_VSCROLL, if total > 0 { SB_PAGEUP } else { SB_PAGEDOWN }, 0);
        }
        return;
    }
    let total = REST.load(Ordering::Relaxed) + delta * lines.max(1) as i32;
    REST.store(total % WHEEL_DELTA, Ordering::Relaxed);
    let steps = total / WHEEL_DELTA;
    if steps != 0 {
        // Hjulet væk fra brugeren (negativ) ruller ned i teksten.
        SendMessageW(edit, EM_LINESCROLL, 0, -steps as LPARAM);
    }
}

/// RichEdit har ingen højrekliksmenu selv, så vi laver den klassiske.
unsafe fn context_menu(edit: HWND, lp: LPARAM) {
    let mut at = POINT { x: lo(lp), y: hi(lp) };
    if lp == -1 {
        GetCursorPos(&mut at);
    }
    let mut selection = CHARRANGE { cpMin: 0, cpMax: 0 };
    SendMessageW(edit, EM_EXGETSEL, 0, &mut selection as *mut CHARRANGE as LPARAM);
    let has_selection = selection.cpMax != selection.cpMin;
    let enabled = |on: bool| if on { MF_STRING } else { MF_STRING | MF_GRAYED };

    let words = crate::lang::s();
    let item = |text: &str| crate::util::wide(text);
    let menu = CreatePopupMenu();
    AppendMenuW(menu, enabled(SendMessageW(edit, EM_CANUNDO, 0, 0) != 0), 1, item(words.undo).as_ptr());
    AppendMenuW(menu, MF_SEPARATOR, 0, null());
    AppendMenuW(menu, enabled(has_selection), 2, item(words.cut).as_ptr());
    AppendMenuW(menu, enabled(has_selection), 3, item(words.copy).as_ptr());
    AppendMenuW(menu, enabled(SendMessageW(edit, EM_CANPASTE, 0, 0) != 0), 4, item(words.paste).as_ptr());
    AppendMenuW(menu, enabled(has_selection), 5, item(words.delete).as_ptr());
    AppendMenuW(menu, MF_SEPARATOR, 0, null());
    AppendMenuW(menu, MF_STRING, 6, item(words.select_all).as_ptr());
    AppendMenuW(menu, MF_SEPARATOR, 0, null());
    let wrap = if crate::ui::word_wrap() { MF_STRING | MF_CHECKED } else { MF_STRING };
    // Genvejene i menuen er dem, der er valgt i indstillingerne.
    let label = |text: &str, command: usize| crate::util::wide(&format!("{text}{}", crate::ui::menu_shortcut(command)));
    AppendMenuW(menu, wrap, 7, label(words.wrap_lines, crate::ui::CMD_WRAP).as_ptr());
    AppendMenuW(menu, MF_STRING, 8, label(words.space_comments, crate::ui::CMD_SPACE_COMMENTS).as_ptr());
    AppendMenuW(menu, MF_SEPARATOR, 0, null());
    AppendMenuW(menu, MF_STRING, 10, label(words.find_replace, crate::ui::CMD_FIND).as_ptr());
    AppendMenuW(menu, MF_STRING, 9, label(words.settings, crate::ui::CMD_SETTINGS).as_ptr());
    let choice = TrackPopupMenu(menu, TPM_RETURNCMD | TPM_RIGHTBUTTON, at.x, at.y, 0, edit, null());
    DestroyMenu(menu);
    match choice {
        10 => PostMessageW(GetAncestor(edit, GA_ROOT), WM_COMMAND, crate::ui::CMD_FIND, 0) as isize,
        9 => PostMessageW(GetAncestor(edit, GA_ROOT), WM_COMMAND, crate::ui::CMD_SETTINGS, 0) as isize,
        8 => PostMessageW(GetAncestor(edit, GA_ROOT), WM_COMMAND, crate::ui::CMD_SPACE_COMMENTS, 0) as isize,
        7 => PostMessageW(GetAncestor(edit, GA_ROOT), WM_COMMAND, crate::ui::CMD_WRAP, 0) as isize,
        1 => SendMessageW(edit, WM_UNDO, 0, 0),
        2 => SendMessageW(edit, WM_CUT, 0, 0),
        3 => SendMessageW(edit, WM_COPY, 0, 0),
        4 => SendMessageW(edit, WM_PASTE, 0, 0),
        5 => SendMessageW(edit, WM_CLEAR, 0, 0),
        6 => SendMessageW(edit, EM_SETSEL, 0, -1),
        _ => 0,
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    fn w(s: &str) -> Vec<u16> {
        s.encode_utf16().collect()
    }

    #[test]
    fn lf_file_round_trips_byte_for_byte() {
        let original = b"A=1\nB=2\n";
        let (text, enc, eol) = decode(original);
        assert_eq!((enc, eol), (Enc::Utf8, Eol::Lf));
        assert_eq!(text, w("A=1\rB=2\r"));
        assert_eq!(encode(&text, enc, eol), original);
    }

    #[test]
    fn crlf_file_round_trips_byte_for_byte() {
        let original = b"A=1\r\nB=2\r\n\r\nC=3";
        let (text, enc, eol) = decode(original);
        assert_eq!((enc, eol), (Enc::Utf8, Eol::Crlf));
        assert_eq!(encode(&text, enc, eol), original);
    }

    #[test]
    fn utf8_bom_is_kept() {
        let original = b"\xEF\xBB\xBFn\xC3\xB8gle=\xC3\xA6\n";
        let (text, enc, eol) = decode(original);
        assert_eq!(enc, Enc::Utf8Bom);
        assert_eq!(text, w("nøgle=æ\r"));
        assert_eq!(encode(&text, enc, eol), original);
    }

    #[test]
    fn utf16_le_and_be_round_trip() {
        let le = [0xFF, 0xFE, b'a', 0, b'\r', 0, b'\n', 0, 0xE6, 0];
        let (text, enc, eol) = decode(&le);
        assert_eq!((enc, eol), (Enc::Utf16Le, Eol::Crlf));
        assert_eq!(text, w("a\ræ"));
        assert_eq!(encode(&text, enc, eol), le);

        let be = [0xFE, 0xFF, 0, b'a', 0, b'\n', 0, 0xE6];
        let (text, enc, eol) = decode(&be);
        assert_eq!((enc, eol), (Enc::Utf16Be, Eol::Lf));
        assert_eq!(encode(&text, enc, eol), be);
    }

    #[test]
    fn invalid_utf8_falls_back_to_ansi_and_round_trips() {
        // 0xE6 alene er "æ" i Windows-1252, men ugyldig UTF-8.
        let original = b"navn=\xE6ble\r\n";
        let (text, enc, eol) = decode(original);
        assert_eq!(enc, Enc::Ansi);
        assert_eq!(encode(&text, enc, eol), original);
    }

    #[test]
    fn file_without_line_breaks_defaults_to_crlf() {
        let (text, enc, eol) = decode(b"A=1");
        assert_eq!((enc, eol), (Enc::Utf8, Eol::Crlf));
        assert_eq!(encode(&w("A=1\rB=2"), enc, eol), b"A=1\r\nB=2");
        assert_eq!(text, w("A=1"));
    }

    /// Kører reglen på tekst med almindelige \n og returnerer resultatet på samme form.
    fn spaced(input: &str) -> String {
        let text = w(&input.replace('\n', "\r"));
        let offsets = blank_line_offsets(&text);
        let mut out = Vec::new();
        for (i, &c) in text.iter().enumerate() {
            if offsets.contains(&i) {
                out.push(CR);
            }
            out.push(c);
        }
        String::from_utf16(&out).unwrap().replace('\r', "\n")
    }

    #[test]
    fn comment_above_text_gets_a_blank_line_under_it() {
        assert_eq!(spaced("# url\nURL=x\nKEY=\n# server\nSECRET=\n"), "# url\n\nURL=x\nKEY=\n# server\n\nSECRET=\n");
    }

    #[test]
    fn comment_without_text_under_it_is_left_alone() {
        // Udkommenterede nøgler er også kommentarer, så blokken har ingen tekst under sig.
        let block = "# Valgfrit: opslag i CVR\n# og adgangskoden\n# VIRK_USER=\n# VIRK_PASSWORD=\n";
        assert_eq!(spaced(block), block);
        assert_eq!(spaced("# sidste linje"), "# sidste linje");
        assert_eq!(spaced("# kun kommentar\n"), "# kun kommentar\n");
    }

    #[test]
    fn trailing_comment_block_is_separated_from_the_text_above_it() {
        // Den sidste nøgle får luft under sig, og kommentarblokken nederst forbliver samlet.
        assert_eq!(
            spaced("# mail\nEMAIL=x\n# Valgfrit\n# VIRK_USER=\n# Kun hvis\n# VIRK_URL=\n"),
            "# mail\n\nEMAIL=x\n\n# Valgfrit\n# VIRK_USER=\n# Kun hvis\n# VIRK_URL=\n"
        );
        assert_eq!(spaced("A=1\n# note\n\n# mere\n"), "A=1\n\n# note\n\n# mere\n");
        assert_eq!(spaced("A=1\n# sidste linje"), "A=1\n\n# sidste linje");
    }

    #[test]
    fn comment_already_spaced_from_its_text_is_not_pushed_away_from_the_text_above() {
        // Sådan ser en blok ud efter første tryk; den har stadig tekst under sig.
        let spaced_once = "A=1\n# om B\n\nB=2\n";
        assert_eq!(spaced(spaced_once), spaced_once);
    }

    #[test]
    fn only_the_last_line_of_a_comment_block_is_spaced() {
        assert_eq!(spaced("# linje 1\n# linje 2\nKEY=1"), "# linje 1\n# linje 2\n\nKEY=1");
    }

    #[test]
    fn running_it_twice_changes_nothing_more() {
        let once = spaced("# a\nA=1\n  # b\n\tB=2\n# c\n   \nC=3\n# slut\n");
        assert_eq!(once, "# a\n\nA=1\n  # b\n\n\tB=2\n# c\n   \nC=3\n\n# slut\n");
        assert_eq!(spaced(&once), once);
    }

    #[test]
    fn semicolon_and_double_slash_count_as_comments() {
        assert_eq!(spaced("; ini\nkey=1\n// kode\nx = 2\n/ ikke kommentar\ny"), "; ini\n\nkey=1\n// kode\n\nx = 2\n/ ikke kommentar\ny");
    }

    #[test]
    fn signature_tells_texts_apart_and_is_stable() {
        assert_eq!(signature(&w("A=1\rB=2")), signature(&w("A=1\rB=2")));
        assert_ne!(signature(&w("A=1\rB=2")), signature(&w("A=1\rB=3")));
        assert_ne!(signature(&w("ab")), signature(&w("ba")));
        // Værdien står i sessionsfiler og må ikke ændre sig mellem versioner.
        assert_eq!(signature(&[]), Sig { len: 0, hash: 0xcbf2_9ce4_8422_2325 });
        assert_eq!(signature(&w("a")), Sig { len: 1, hash: 0x089b_e207_b544_f1e4 });
    }

    #[test]
    fn soft_line_break_is_saved_as_a_real_one() {
        assert_eq!(encode(&[b'a' as u16, VT, b'b' as u16], Enc::Utf8, Eol::Lf), b"a\nb");
    }
}
