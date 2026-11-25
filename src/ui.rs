//! Vinduer, fanelinje, genveje, træk-ud af faner og genindlæsning af ændrede filer.
//!
//! Alt kører på én tråd. Tilstanden ligger i en enkelt `App`, som vinduesprocedurerne
//! slår op i. Win32 kalder tilbage i procedurerne midt i mange kald (SendMessage,
//! ShowWindow, MessageBox ...), så koden slår vindue og dokument op igen efter
//! den slags kald i stedet for at holde på referencer.

use std::ffi::OsString;
use std::mem::{size_of, zeroed};
use std::os::windows::ffi::OsStringExt;
use std::path::{Path, PathBuf};
use std::ptr::{null, null_mut};
use std::sync::atomic::{AtomicPtr, Ordering};

use windows_sys::w;
use windows_sys::Win32::Foundation::{HANDLE, HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT, SIZE, WPARAM};
use windows_sys::Win32::Graphics::Dwm::DwmSetWindowAttribute;
use windows_sys::Win32::Graphics::Gdi::{
    BeginPaint, BitBlt, ClientToScreen, CreateCompatibleBitmap, CreateCompatibleDC, CreateFontW,
    DeleteDC, DeleteObject, DrawTextW, EndPaint, ExtTextOutW, GetDC, GetTextExtentPoint32W,
    InvalidateRect, PtInRect, RedrawWindow, ReleaseDC, ScreenToClient, SelectObject, SetBkColor,
    SetBkMode, SetTextColor, DT_CENTER, DT_END_ELLIPSIS, DT_NOPREFIX, DT_SINGLELINE, DT_VCENTER,
    ETO_OPAQUE, HDC, HFONT, PAINTSTRUCT, RDW_ALLCHILDREN, RDW_ERASE, RDW_FRAME, RDW_INVALIDATE, RDW_UPDATENOW,
    SRCCOPY, TRANSPARENT,
};
use windows_sys::Win32::System::DataExchange::COPYDATASTRUCT;
use windows_sys::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress, LoadLibraryW};
use windows_sys::Win32::System::Registry::{
    RegGetValueW, RegSetKeyValueW, HKEY_CURRENT_USER, REG_DWORD, RRF_RT_REG_DWORD,
};
use windows_sys::Win32::UI::Controls::Dialogs::{
    GetOpenFileNameW, GetSaveFileNameW, OFN_ALLOWMULTISELECT, OFN_EXPLORER, OFN_FILEMUSTEXIST,
    OFN_HIDEREADONLY, OFN_NOCHANGEDIR, OFN_OVERWRITEPROMPT, OPENFILENAMEW,
};
use windows_sys::Win32::UI::HiDpi::{GetDpiForSystem, GetDpiForWindow};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    GetActiveWindow, GetAsyncKeyState, GetKeyState, ReleaseCapture, SetCapture, SetFocus,
    TrackMouseEvent, TME_LEAVE, TRACKMOUSEEVENT, VK_CONTROL, VK_ESCAPE, VK_INSERT, VK_LBUTTON, VK_MENU,
    VK_OEM_COMMA, VK_OEM_PERIOD, VK_OEM_PLUS, VK_RBUTTON, VK_RETURN, VK_SHIFT, VK_TAB,
};
use windows_sys::Win32::UI::Shell::{DragFinish, DragQueryFileW, HDROP};
use windows_sys::Win32::UI::WindowsAndMessaging::*;

use crate::dialog::{self, Choice};
use crate::doc::{self, Doc, Enc, Eol};
use crate::find;
use crate::highlight;
use crate::lang;
use crate::syntax::Lang;
use crate::session::{self, Snapshot};
use crate::settings::{self, Key, Settings};
use crate::theme::Theme;
use crate::util::{hi, lo, make_lparam, scale, wide, wide_os};
use crate::watch::{Watcher, MAX_WATCHES};

pub const IPC_MAGIC: usize = 0x7345_6469;
const WINDOW_CLASS: *const u16 = w!("sEdit.Window");
const WM_APP_OPEN: u32 = WM_APP + 1;
const TIMER_CHECK: usize = 1;
const DWMWA_USE_IMMERSIVE_DARK_MODE: u32 = 20;
const HTCAPTION_HIT: WPARAM = 2;
// Ligger uden for WindowsAndMessaging i windows-sys.
const WM_MOUSELEAVE: u32 = 0x02A3;
const INFINITE: u32 = u32::MAX;

pub const CMD_SAVE: usize = 1;
pub const CMD_SAVE_AS: usize = 2;
pub const CMD_NEW: usize = 3;
pub const CMD_OPEN: usize = 4;
pub const CMD_CLOSE: usize = 5;
pub const CMD_NEXT: usize = 6;
pub const CMD_PREV: usize = 7;
pub const CMD_WRAP: usize = 8;
pub const CMD_SPACE_COMMENTS: usize = 9;
pub const CMD_SETTINGS: usize = 10;
pub const CMD_FIND: usize = 11;
pub const CMD_FIND_NEXT: usize = 12;
pub const CMD_FIND_PREV: usize = 13;
const CMD_GOTO: usize = 100;

const GLYPH_CLOSE: u16 = 0xE8BB;
const GLYPH_PLUS: u16 = 0xE710;
const GLYPH_SETTINGS: u16 = 0xE713;
const GLYPH_DIRTY: u16 = 0x25CF;

#[derive(Clone, Copy, PartialEq)]
enum Hit {
    None,
    Tab(usize),
    Close(usize),
    Plus,
    Settings,
    /// En knap på søgelinjen.
    Find(FindButton),
}

/// En fane der er taget fat i med musen men endnu ikke trukket ud af fanelinjen.
struct Drag {
    idx: usize,
    start: POINT,
    /// Hvor på fanen der blev taget fat, så et udtrukket vindue lander under markøren.
    grab: POINT,
    moved: bool,
}

struct Win {
    hwnd: HWND,
    docs: Vec<Doc>,
    active: usize,
    dpi: u32,
    hover: Hit,
    press: Hit,
    drag: Option<Drag>,
    /// Indsætningsposition der vises, mens en fane fra et andet vindue holdes over fanelinjen.
    drop_hint: Option<usize>,
    /// Vinduet flyttes rundt af en fane-trækning og kan slippes på et andet vindue.
    floating: bool,
    over_target: bool,
    tracking: bool,
    /// Bredden af margenen med linjenumre; 0 når de er slået fra.
    gutter: i32,
    /// Søgelinjen, når den er åben.
    find: Option<FindBar>,
}

#[derive(Clone, Copy)]
struct Fonts {
    dpi: u32,
    ui: HFONT,
    icon: HFONT,
    edit: HFONT,
}

struct App {
    hinst: HINSTANCE,
    wins: Vec<Win>,
    fonts: Vec<Fonts>,
    theme: Theme,
    /// Det brugeren har valgt i indstillingerne; huskes mellem sessioner.
    settings: Settings,
    next_id: u32,
    untitled: u32,
    ipc: HWND,
    last_active: HWND,
    watch: Watcher,
    pending: Vec<PathBuf>,
    timer_armed: bool,
    checking: bool,
    recheck: bool,
    retries: u32,
    session_dir: PathBuf,
    /// Faner fra vinduer der er lukket i denne kørsel. De kommer med i sessionen, når
    /// appen lukker, og hentes frem igen, hvis filen åbnes forinden.
    parked: Vec<Snapshot>,
    /// Sessionen er allerede skrevet af det vindue, der lukkede sidst.
    session_written: bool,
    /// Gendannede faner hvor filen er ændret siden kladden blev gemt; brugeren skal vælge.
    conflicts: Vec<Conflict>,
}

struct Conflict {
    id: u32,
    draft: Vec<u16>,
    selection: (i32, i32),
}

static APP: AtomicPtr<App> = AtomicPtr::new(null_mut());

fn app() -> &'static mut App {
    unsafe { &mut *APP.load(Ordering::Relaxed) }
}

fn win(hwnd: HWND) -> Option<&'static mut Win> {
    if APP.load(Ordering::Relaxed).is_null() {
        return None;
    }
    app().wins.iter_mut().find(|w| w.hwnd == hwnd)
}

fn find_doc(id: u32) -> Option<(&'static mut Win, usize)> {
    for w in app().wins.iter_mut() {
        if let Some(i) = w.docs.iter().position(|d| d.id == id) {
            return Some((w, i));
        }
    }
    None
}

unsafe fn fonts(dpi: u32) -> Fonts {
    let a = app();
    if let Some(f) = a.fonts.iter().find(|f| f.dpi == dpi) {
        return *f;
    }
    let points = |pt: i32| -((pt * dpi as i32 + 36) / 72);
    let make = |height: i32, face: &str, pitch: u32| {
        let face = wide(face);
        CreateFontW(height, 0, 0, 0, 400, 0, 0, 0, 1, 0, 0, 5, pitch, face.as_ptr())
    };
    let f = Fonts {
        dpi,
        ui: make(points(9), "Segoe UI", 0),
        icon: make(-scale(dpi, 10), "Segoe MDL2 Assets", 0),
        edit: make(points(a.settings.size as i32), a.settings.font_face(), 1 | 48),
    };
    a.fonts.push(f);
    f
}

pub fn theme() -> Theme {
    app().theme
}

pub fn word_wrap() -> bool {
    app().settings.wrap
}

pub unsafe fn ui_font(dpi: u32) -> HFONT {
    fonts(dpi).ui
}

pub unsafe fn icon_font(dpi: u32) -> HFONT {
    fonts(dpi).icon
}

pub fn settings() -> &'static mut Settings {
    &mut app().settings
}

/// Genvejen til en kommando som den skrives i en menu: tabulator og "Ctrl+S", eller
/// ingenting hvis kommandoen ikke har en genvej.
pub fn menu_shortcut(command: usize) -> String {
    let shortcut = app().settings.shortcut(command);
    if shortcut.is_empty() {
        shortcut
    } else {
        format!("\t{shortcut}")
    }
}

/// Sproget en fil farves som, eller None når syntaksfarver er slået fra.
fn lang_for(path: Option<&Path>) -> Option<Lang> {
    path.filter(|_| app().settings.syntax).and_then(Lang::of)
}

/// Hvad der er ændret i indstillingerne, så kun det berørte laves om.
#[derive(Clone, Copy, PartialEq)]
pub enum Change {
    Theme,
    Font,
    Wrap,
    Syntax,
    /// "Kun hele ord" i søgningen er slået til eller fra.
    Find,
    /// Linjenumrene er slået til eller fra.
    Gutter,
    Language,
    All,
}

/// Lader en ændret indstilling slå igennem i alle åbne vinduer og faner.
pub unsafe fn apply_settings(change: Change) {
    let all = change == Change::All;
    let windows: Vec<HWND> = app().wins.iter().map(|w| w.hwnd).collect();
    if all || change == Change::Font {
        // Tekstfelternes skrift laves om for hver skærmskalering, der er i brug.
        let a = app();
        let (face, size) = (wide(a.settings.font_face()), a.settings.size as i32);
        for f in a.fonts.iter_mut() {
            DeleteObject(f.edit);
            let height = -((size * f.dpi as i32 + 36) / 72);
            f.edit = CreateFontW(height, 0, 0, 0, 400, 0, 0, 0, 1, 0, 0, 5, 1 | 48, face.as_ptr());
        }
    }
    if all || change == Change::Language {
        lang::select(app().settings.language);
        for &hwnd in &windows {
            // Søgelinjens knapper følger teksternes bredde.
            place_find_inputs(hwnd);
            RedrawWindow(hwnd, null(), null_mut(), RDW_INVALIDATE | RDW_ERASE | RDW_ALLCHILDREN);
        }
    }
    if all || change == Change::Theme {
        app().theme = Theme::for_mode(app().settings.theme);
        allow_dark_menus();
    }
    if all || change == Change::Theme || change == Change::Font {
        for &hwnd in &windows {
            apply_frame_theme(hwnd);
            restyle(hwnd);
            RedrawWindow(hwnd, null(), null_mut(), RDW_INVALIDATE | RDW_ERASE | RDW_FRAME | RDW_ALLCHILDREN);
        }
    }
    if all || change == Change::Gutter || change == Change::Font {
        // Margenen med linjenumre kommer, går eller skifter bredde med skriften.
        for &hwnd in &windows {
            layout(hwnd);
        }
    }
    if all || change == Change::Find || change == Change::Language {
        // Åbne søgelinjer søger igen med den nye regel.
        for &hwnd in &windows {
            if win(hwnd).is_some_and(|w| w.find.is_some()) {
                find_step(hwnd, 0);
            }
        }
    }
    let docs: Vec<(HWND, Option<PathBuf>)> = app().wins.iter().flat_map(|w| w.docs.iter().map(|d| (d.edit, d.path.clone()))).collect();
    for (edit, path) in docs {
        if all || change == Change::Wrap {
            doc::set_wrap(edit, app().settings.wrap);
        }
        if all || change == Change::Syntax {
            highlight::set_lang(edit, lang_for(path.as_deref()));
        }
    }
}

/// Viser et nyt vindue uden det hvide blink. Windows viser ellers vinduets tomme, hvide
/// flade i et øjeblik, før appen har tegnet første gang. Vinduet holdes derfor skjult for
/// skærmen (cloaked), til det og tekstfeltet er tegnet færdigt.
unsafe fn show_painted(hwnd: HWND, command: i32) {
    const DWMWA_CLOAK: u32 = 13;
    let cloak = |on: i32| DwmSetWindowAttribute(hwnd, DWMWA_CLOAK, &on as *const i32 as *const _, 4);
    cloak(1);
    ShowWindow(hwnd, command);
    RedrawWindow(hwnd, null(), null_mut(), RDW_INVALIDATE | RDW_UPDATENOW | RDW_ALLCHILDREN);
    cloak(0);
}

/// Mørk titellinje, når temaet er mørkt.
pub unsafe fn apply_frame_theme(hwnd: HWND) {
    let dark: i32 = app().theme.dark as i32;
    DwmSetWindowAttribute(hwnd, DWMWA_USE_IMMERSIVE_DARK_MODE, &dark as *const i32 as *const _, 4);
}

/// Lader højrekliksmenuer og fildialoger følge Windows' mørke tema.
///
/// Windows har ingen dokumenteret måde at bede om mørke menuer på. De to funktioner
/// eksporteres kun som numre fra uxtheme.dll, og det er dem Stifinder og Notepad selv
/// bruger. Findes de ikke, sker der ingenting, og menuerne forbliver lyse.
unsafe fn allow_dark_menus() {
    const SET_PREFERRED_APP_MODE: usize = 135;
    const FLUSH_MENU_THEMES: usize = 136;
    // 1 følger Windows, 2 er altid mørk, 3 er altid lys.
    let mode: i32 = match app().settings.theme {
        1 => 3,
        2 => 2,
        _ => 1,
    };
    let uxtheme = LoadLibraryW(w!("uxtheme.dll"));
    if uxtheme.is_null() {
        return;
    }
    if let Some(set_mode) = GetProcAddress(uxtheme, SET_PREFERRED_APP_MODE as *const u8) {
        let set_mode: unsafe extern "system" fn(i32) -> i32 = std::mem::transmute(set_mode);
        set_mode(mode);
    }
    if let Some(flush) = GetProcAddress(uxtheme, FLUSH_MENU_THEMES as *const u8) {
        let flush: unsafe extern "system" fn() = std::mem::transmute(flush);
        flush();
    }
}

// ---------- indstillinger ----------

/// Nøglen i registreringsdatabasen. En testinstans (SEDIT_INSTANCE) får sin egen, så
/// den aldrig retter i de indstillinger man selv bruger.
fn settings_key() -> *const u16 {
    static KEY: std::sync::OnceLock<Vec<u16>> = std::sync::OnceLock::new();
    KEY.get_or_init(|| wide(&format!("Software\\sEdit{}", std::env::var("SEDIT_INSTANCE").unwrap_or_default()))).as_ptr()
}

pub unsafe fn read_setting(name: *const u16) -> Option<u32> {
    let mut value: u32 = 0;
    let mut size: u32 = 4;
    let status = RegGetValueW(
        HKEY_CURRENT_USER,
        settings_key(),
        name,
        RRF_RT_REG_DWORD,
        null_mut(),
        &mut value as *mut u32 as *mut _,
        &mut size,
    );
    (status == 0).then_some(value)
}

pub unsafe fn write_setting(name: *const u16, value: u32) {
    RegSetKeyValueW(HKEY_CURRENT_USER, settings_key(), name, REG_DWORD, &value as *const u32 as *const _, 4);
}

// ---------- opstart og hovedløkke ----------

/// `ipc_class` er navnet på det skjulte vindue, som senere startede sEdit-processer
/// finder og afleverer deres filer til.
pub unsafe fn run(paths: Vec<PathBuf>, ipc_class: &[u16], session_dir: PathBuf) {
    let hinst = GetModuleHandleW(null());
    LoadLibraryW(w!("Msftedit.dll"));

    let settings = Settings::load();
    lang::select(settings.language);
    let state = Box::new(App {
        hinst,
        wins: Vec::new(),
        fonts: Vec::new(),
        theme: Theme::for_mode(settings.theme),
        settings,
        next_id: 0,
        untitled: 0,
        ipc: null_mut(),
        last_active: null_mut(),
        watch: Watcher::new(),
        pending: Vec::new(),
        timer_armed: false,
        checking: false,
        recheck: false,
        retries: 0,
        session_dir,
        parked: Vec::new(),
        session_written: false,
        conflicts: Vec::new(),
    });
    APP.store(Box::into_raw(state), Ordering::Relaxed);
    allow_dark_menus();

    let mut class: WNDCLASSEXW = zeroed();
    class.cbSize = size_of::<WNDCLASSEXW>() as u32;
    class.style = CS_DBLCLKS;
    class.lpfnWndProc = Some(window_proc);
    class.hInstance = hinst;
    class.hCursor = LoadCursorW(null_mut(), IDC_ARROW);
    // Ikonet er indlejret af build.rs som ressource nr. 1. Mangler det, bruges Windows' standardikon.
    let icon = |cx: i32, cy: i32| {
        LoadImageW(hinst, 1 as *const u16, IMAGE_ICON, GetSystemMetrics(cx), GetSystemMetrics(cy), LR_SHARED)
    };
    let large = icon(SM_CXICON, SM_CYICON);
    class.hIcon = if large.is_null() { LoadIconW(null_mut(), IDI_APPLICATION) } else { large };
    class.hIconSm = icon(SM_CXSMICON, SM_CYSMICON);
    class.lpszClassName = WINDOW_CLASS;
    RegisterClassExW(&class);

    let mut ipc_window: WNDCLASSEXW = zeroed();
    ipc_window.cbSize = size_of::<WNDCLASSEXW>() as u32;
    ipc_window.lpfnWndProc = Some(ipc_proc);
    ipc_window.hInstance = hinst;
    ipc_window.lpszClassName = ipc_class.as_ptr();
    RegisterClassExW(&ipc_window);
    app().ipc = CreateWindowExW(0, ipc_class.as_ptr(), null(), 0, 0, 0, 0, 0, HWND_MESSAGE, null_mut(), hinst, null());

    let hwnd = create_window(None);
    restore_session(hwnd);
    open_paths(hwnd, &paths);
    if win(hwnd).is_some_and(|w| w.docs.is_empty()) {
        new_tab(hwnd);
    }
    // SW_SHOWDEFAULT respekterer genvejens "Kør: minimeret/maksimeret".
    show_painted(hwnd, SW_SHOWDEFAULT);
    resolve_conflicts();

    let mut msg: MSG = zeroed();
    'run: loop {
        let mut handles: [HANDLE; MAX_WATCHES] = [null_mut(); MAX_WATCHES];
        let count = app().watch.fill(&mut handles) as u32;
        let woke = MsgWaitForMultipleObjectsEx(count, handles.as_ptr(), INFINITE, QS_ALLINPUT, MWMO_INPUTAVAILABLE);
        if woke < count {
            app().watch.rearm(woke as usize);
            arm_check(20);
        } else if woke == u32::MAX {
            // Et håndtag er blevet ugyldigt. Drop dem alle; de genskabes ved næste afstemning.
            app().watch.clear();
        }
        while PeekMessageW(&mut msg, null_mut(), 0, 0, PM_REMOVE) != 0 {
            if msg.message == WM_QUIT {
                break 'run;
            }
            if !pre_translate(&msg) {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
            if !matches!(msg.message, WM_MOUSEMOVE | WM_NCMOUSEMOVE | WM_PAINT | WM_TIMER) {
                sync_dirty(GetAncestor(msg.hwnd, GA_ROOT));
            }
        }
    }
}

/// Genveje fanges før tekstfeltet ser tasten. Returnerer true hvis tasten er brugt.
unsafe fn pre_translate(msg: &MSG) -> bool {
    if msg.message != WM_KEYDOWN && msg.message != WM_SYSKEYDOWN {
        return false;
    }
    let down = |vk: u16| GetKeyState(vk as i32) < 0;
    let (ctrl, alt, shift) = (down(VK_CONTROL), down(VK_MENU), down(VK_SHIFT));
    let root = GetAncestor(msg.hwnd, GA_ROOT);
    if win(root).is_none() {
        return false;
    }
    let key = msg.wParam as u16;
    let is = |c: u8| key == c as u16;

    // Enter, Tab og Esc i søgelinjen.
    if msg.message == WM_KEYDOWN && !ctrl && !alt && find_key(root, msg.hwnd, key, shift) {
        return true;
    }
    // Genvejene er dem brugeren har valgt i indstillingerne.
    if let Some(cmd) = app().settings.command_for(Key::new(key, ctrl, alt, shift)) {
        command(root, cmd);
        return true;
    }
    // RichEdit indsætter selv ved Ctrl+V og Shift+Insert, med formatering og det hele.
    // Tasterne sendes i stedet som WM_PASTE, som tekstfeltet gør til ren tekst.
    let paste = if ctrl { is(b'V') && !alt } else { shift && !alt && key == VK_INSERT };
    if paste && win(root).is_some_and(|w| w.docs.iter().any(|d| d.edit == msg.hwnd)) {
        SendMessageW(msg.hwnd, WM_PASTE, 0, 0);
        return true;
    }
    // Ctrl+Alt er AltGr på dansk tastatur (@, €, {, } ...) og skal ikke røres.
    if !ctrl || alt {
        return false;
    }

    let cmd = match key {
        // Ctrl+1 til Ctrl+9 vælger en fane og kan ikke ændres.
        _ if !shift && (b'1' as u16..=b'9' as u16).contains(&key) => CMD_GOTO + (key - b'1' as u16) as usize,
        // Ctrl+Tab må ikke nå tekstfeltet, selv om genvejen er fjernet; der ville den
        // indsætte en tabulator.
        VK_TAB => return true,
        // RichEdits formateringsgenveje (fed, kursiv, centrering ...) giver ingen
        // mening i ren tekst og sluges, så de ikke kan ændre visningen.
        _ if b"BIUERLJ".iter().any(|&c| is(c)) || (shift && is(b'A')) => return true,
        // Hævet/sænket skrift (Ctrl+=) og skriftstørrelse (Ctrl+Shift+, og .).
        VK_OEM_PLUS => return true,
        VK_OEM_COMMA | VK_OEM_PERIOD if shift => return true,
        _ => return false,
    };
    command(root, cmd);
    true
}

/// Udfører en kommando i et vindue. Genvejene ender her, og det samme gør WM_COMMAND.
unsafe fn command(hwnd: HWND, cmd: usize) {
    let Some(w) = win(hwnd) else { return };
    let Some(id) = w.docs.get(w.active).map(|d| d.id) else { return };
    let count = w.docs.len();
    match cmd {
        CMD_SAVE => {
            save_doc(hwnd, id, false);
        }
        CMD_SAVE_AS => {
            save_doc(hwnd, id, true);
        }
        CMD_NEW => new_tab(hwnd),
        CMD_OPEN => {
            let paths = open_dialog(hwnd);
            open_paths(hwnd, &paths);
        }
        CMD_CLOSE => {
            close_tab(hwnd, id);
        }
        CMD_NEXT => step_tab(hwnd, 1),
        CMD_PREV => step_tab(hwnd, -1),
        CMD_WRAP => {
            let a = app();
            a.settings.wrap = !a.settings.wrap;
            a.settings.save();
            apply_settings(Change::Wrap);
        }
        CMD_SETTINGS => settings::show(hwnd),
        CMD_FIND => find_show(hwnd),
        // Uden en åben søgelinje er der intet at gå videre til, så den åbnes.
        CMD_FIND_NEXT | CMD_FIND_PREV if w.find.is_none() => find_show(hwnd),
        CMD_FIND_NEXT => find_step(hwnd, 1),
        CMD_FIND_PREV => find_step(hwnd, -1),
        CMD_SPACE_COMMENTS => {
            doc::space_out_comments(w.docs[w.active].edit);
        }
        // Ctrl+1..8 vælger fane nummer 1..8, Ctrl+9 den sidste.
        _ if cmd == CMD_GOTO + 8 => activate(hwnd, count - 1),
        _ if (CMD_GOTO..CMD_GOTO + 8).contains(&cmd) => activate(hwnd, cmd - CMD_GOTO),
        _ => {}
    }
}

// ---------- vinduer ----------

unsafe fn create_window(rect: Option<RECT>) -> HWND {
    let a = app();
    let system_dpi = GetDpiForSystem() as i32;
    let (x, y, width, height) = match rect {
        Some(r) => (r.left, r.top, r.right - r.left, r.bottom - r.top),
        None => (CW_USEDEFAULT, 0, 1000 * system_dpi / 96, 680 * system_dpi / 96),
    };
    let hwnd = CreateWindowExW(
        WS_EX_ACCEPTFILES,
        WINDOW_CLASS,
        w!("sEdit"),
        WS_OVERLAPPEDWINDOW | WS_CLIPCHILDREN,
        x,
        y,
        width,
        height,
        null_mut(),
        null_mut(),
        a.hinst,
        null(),
    );
    app().wins.push(Win {
        hwnd,
        docs: Vec::new(),
        active: 0,
        dpi: GetDpiForWindow(hwnd),
        hover: Hit::None,
        press: Hit::None,
        drag: None,
        drop_hint: None,
        floating: false,
        over_target: false,
        tracking: false,
        gutter: 0,
        find: None,
    });
    apply_frame_theme(hwnd);
    hwnd
}

unsafe extern "system" fn window_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_PAINT => {
            paint(hwnd);
            0
        }
        WM_ERASEBKGND => 1,
        WM_SIZE => {
            layout(hwnd);
            0
        }
        WM_SETFOCUS => {
            if let Some(d) = win(hwnd).and_then(|w| w.docs.get(w.active)) {
                SetFocus(d.edit);
            }
            0
        }
        WM_ACTIVATE => {
            if lo(wp as isize) != 0 {
                app().last_active = hwnd;
                // Sikkerhedsnet: tjek filerne, hver gang vinduet får fokus.
                arm_check(30);
            }
            DefWindowProcW(hwnd, msg, wp, lp)
        }
        WM_MOUSEMOVE => {
            on_mouse_move(hwnd, lo(lp), hi(lp));
            0
        }
        WM_MOUSELEAVE => {
            if let Some(w) = win(hwnd) {
                w.tracking = false;
                if w.hover != Hit::None {
                    w.hover = Hit::None;
                    invalidate_strip(hwnd);
                }
            }
            0
        }
        WM_LBUTTONDOWN | WM_LBUTTONDBLCLK => {
            on_left_down(hwnd, lo(lp), hi(lp), msg == WM_LBUTTONDBLCLK);
            0
        }
        WM_LBUTTONUP => {
            on_left_up(hwnd, lo(lp), hi(lp));
            0
        }
        WM_MBUTTONUP => {
            if let Some(w) = win(hwnd) {
                if let Hit::Tab(i) | Hit::Close(i) = hit(w, lo(lp), hi(lp)) {
                    let id = w.docs[i].id;
                    close_tab(hwnd, id);
                }
            }
            0
        }
        WM_CAPTURECHANGED => {
            if let Some(w) = win(hwnd) {
                w.drag = None;
            }
            0
        }
        WM_MOVING => {
            if win(hwnd).is_some_and(|w| w.floating) {
                update_drop_hint(hwnd);
            }
            DefWindowProcW(hwnd, msg, wp, lp)
        }
        WM_DPICHANGED => {
            let dpi = ((wp >> 16) & 0xFFFF) as u32;
            let suggested = &*(lp as *const RECT);
            if let Some(w) = win(hwnd) {
                w.dpi = dpi;
            }
            restyle(hwnd);
            SetWindowPos(
                hwnd,
                null_mut(),
                suggested.left,
                suggested.top,
                suggested.right - suggested.left,
                suggested.bottom - suggested.top,
                SWP_NOZORDER | SWP_NOACTIVATE,
            );
            0
        }
        WM_SETTINGCHANGE => {
            let theme = Theme::for_mode(app().settings.theme);
            if theme.dark != app().theme.dark {
                app().theme = theme;
                allow_dark_menus();
                let all: Vec<HWND> = app().wins.iter().map(|w| w.hwnd).collect();
                for h in all {
                    apply_frame_theme(h);
                    restyle(h);
                    RedrawWindow(h, null(), null_mut(), RDW_INVALIDATE | RDW_ERASE | RDW_FRAME | RDW_ALLCHILDREN);
                }
            }
            DefWindowProcW(hwnd, msg, wp, lp)
        }
        WM_DROPFILES => {
            on_drop_files(hwnd, wp as HDROP);
            0
        }
        WM_ENDSESSION => {
            // Windows lukker ned eller logger af: husk alt, mens vinduerne stadig findes.
            if wp != 0 && app().wins.first().is_some_and(|w| w.hwnd == hwnd) {
                save_all_windows();
            }
            0
        }
        WM_COMMAND => {
            // Tekstfelterne sender også WM_COMMAND (med sig selv i lParam); dem ignorerer vi.
            if lp == 0 && (wp >> 16) == 0 {
                command(hwnd, wp & 0xFFFF);
            } else if (wp >> 16) & 0xFFFF == EN_CHANGE && win(hwnd).and_then(|w| w.find.as_ref()).is_some_and(|bar| bar.find == lp as HWND) {
                // Der er skrevet i søgefeltet: gå til det første fund med det samme.
                find_step(hwnd, 0);
            }
            0
        }
        WM_CLOSE => {
            close_window(hwnd);
            0
        }
        WM_DESTROY => {
            let a = app();
            a.wins.retain(|w| w.hwnd != hwnd);
            if a.last_active == hwnd {
                a.last_active = a.wins.first().map_or(null_mut(), |w| w.hwnd);
            }
            sync_watch();
            let a = app();
            if a.wins.is_empty() {
                // Sidste fane blev lukket med Ctrl+W: sessionen skal stadig skrives, ellers
                // dukker den forrige sessions faner op igen ved næste start.
                if !a.session_written {
                    let _ = session::save(&a.session_dir, &a.parked);
                }
                PostQuitMessage(0);
            }
            0
        }
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}

/// Giver alle vinduets tekstfelter skrifttype og farver for vinduets DPI og aktuelle tema.
unsafe fn restyle(hwnd: HWND) {
    let Some(w) = win(hwnd) else { return };
    let dpi = w.dpi;
    let font = fonts(dpi).edit;
    let theme = app().theme;
    for d in w.docs.iter_mut() {
        doc::style_edit(d.edit, font, &theme);
        d.dpi = dpi;
        d.label_w = 0;
    }
    invalidate_strip(hwnd);
    place_find_inputs(hwnd);
}

unsafe fn layout(hwnd: HWND) {
    let Some(w) = win(hwnd) else { return };
    let mut rc: RECT = zeroed();
    GetClientRect(hwnd, &mut rc);
    let top = w.content_top();
    // Margenen med linjenumre ligger til venstre for tekstfeltet.
    let gutter = gutter_width(w);
    w.gutter = gutter;
    if let Some(d) = w.docs.get(w.active) {
        SetWindowPos(d.edit, null_mut(), gutter, top, (rc.right - gutter).max(0), (rc.bottom - top).max(0), SWP_NOZORDER | SWP_NOACTIVATE);
    }
    invalidate_strip(hwnd);
    invalidate_gutter(hwnd);
    place_find_inputs(hwnd);
}

// ---------- linjenumre ----------

const EM_GETLINECOUNT: u32 = 0x00BA;

/// Bredden af et ciffer i tekstfeltets skrift.
unsafe fn digit_width(w: &Win) -> i32 {
    let hdc = GetDC(w.hwnd);
    let previous = SelectObject(hdc, fonts(w.dpi).edit);
    let mut size = SIZE { cx: 0, cy: 0 };
    GetTextExtentPoint32W(hdc, w!("0"), 1, &mut size);
    SelectObject(hdc, previous);
    ReleaseDC(w.hwnd, hdc);
    size.cx.max(1)
}

/// Hvor bred margenen skal være: plads til det største linjenummer (mindst tre cifre)
/// og lidt luft på hver side. 0 når linjenumre er slået fra.
unsafe fn gutter_width(w: &Win) -> i32 {
    let Some(d) = w.docs.get(w.active) else { return 0 };
    if !app().settings.line_numbers {
        return 0;
    }
    let lines = SendMessageW(d.edit, EM_GETLINECOUNT, 0, 0).max(1);
    let digits = (lines.to_string().len() as i32).max(3);
    (digits + 2) * digit_width(w)
}

unsafe fn invalidate_gutter(hwnd: HWND) {
    let Some(w) = win(hwnd) else { return };
    if w.gutter > 0 {
        let mut rc: RECT = zeroed();
        GetClientRect(hwnd, &mut rc);
        let area = RECT { left: 0, top: w.content_top(), right: w.gutter, bottom: rc.bottom };
        InvalidateRect(hwnd, &area, 0);
    }
}

/// Kaldes når et tekstfelt har tegnet sig. Er det den aktive fane, tegnes linjenumrene
/// om, og margenen gøres bredere eller smallere, hvis antallet af cifre har ændret sig.
pub unsafe fn edit_painted(edit: HWND) {
    if !app().settings.line_numbers {
        return;
    }
    let hwnd = GetParent(edit);
    let Some(w) = win(hwnd) else { return };
    if w.docs.get(w.active).map(|d| d.edit) != Some(edit) {
        return;
    }
    if gutter_width(w) != w.gutter {
        layout(hwnd);
    } else {
        invalidate_gutter(hwnd);
    }
}

unsafe fn draw_gutter(hdc: HDC, w: &Win, width: i32, height: i32) {
    let theme = app().theme;
    fill(hdc, &RECT { left: 0, top: 0, right: width, bottom: height }, theme.editor_bg);
    let Some(d) = w.docs.get(w.active) else { return };
    SelectObject(hdc, fonts(w.dpi).edit);
    SetBkMode(hdc, TRANSPARENT as _);
    SetTextColor(hdc, theme.dim);
    // Tallene står højrestillet med ét ciffers luft til teksten.
    let right = width - digit_width(w);
    for (y, number) in highlight::visible_lines(d.edit) {
        let text: Vec<u16> = number.to_string().encode_utf16().collect();
        let mut size = SIZE { cx: 0, cy: 0 };
        GetTextExtentPoint32W(hdc, text.as_ptr(), text.len() as i32, &mut size);
        ExtTextOutW(hdc, right - size.cx, y, 0, null(), text.as_ptr(), text.len() as u32, null());
    }
}

unsafe fn invalidate_strip(hwnd: HWND) {
    let Some(w) = win(hwnd) else { return };
    let mut rc: RECT = zeroed();
    GetClientRect(hwnd, &mut rc);
    // Søgelinjen ligger lige under fanelinjen og tegnes om sammen med den.
    rc.bottom = w.content_top();
    InvalidateRect(hwnd, &rc, 0);
}

unsafe fn update_title(hwnd: HWND) {
    let Some(w) = win(hwnd) else { return };
    let Some(d) = w.docs.get(w.active) else { return };
    let shown = match &d.path {
        Some(p) => p.display().to_string(),
        None => d.name.clone(),
    };
    SetWindowTextW(hwnd, wide(&format!("{shown} – sEdit")).as_ptr());
}

/// Viser den aktive fanes tekstfelt og skjuler resten.
unsafe fn show_active(hwnd: HWND) {
    let Some(w) = win(hwnd) else { return };
    if w.docs.is_empty() {
        return;
    }
    w.active = w.active.min(w.docs.len() - 1);
    let active_edit = w.docs[w.active].edit;
    let others: Vec<HWND> = w.docs.iter().map(|d| d.edit).filter(|&e| e != active_edit).collect();
    layout(hwnd);
    ShowWindow(active_edit, SW_SHOW);
    for edit in others {
        ShowWindow(edit, SW_HIDE);
    }
    if GetActiveWindow() == hwnd {
        SetFocus(active_edit);
    }
    update_title(hwnd);
    invalidate_strip(hwnd);
}

unsafe fn activate(hwnd: HWND, index: usize) {
    let Some(w) = win(hwnd) else { return };
    if index >= w.docs.len() {
        return;
    }
    w.active = index;
    show_active(hwnd);
}

unsafe fn step_tab(hwnd: HWND, delta: isize) {
    let Some(w) = win(hwnd) else { return };
    let count = w.docs.len() as isize;
    if count > 1 {
        let next = (w.active as isize + delta).rem_euclid(count);
        activate(hwnd, next as usize);
    }
}

unsafe fn bring_to_front(hwnd: HWND) {
    if IsIconic(hwnd) != 0 {
        ShowWindow(hwnd, SW_RESTORE);
    }
    SetForegroundWindow(hwnd);
}

// ---------- fanelinjen ----------

impl Win {
    fn strip_height(&self) -> i32 {
        scale(self.dpi, 36)
    }

    /// Hvor teksten begynder: under fanelinjen og søgelinjen, hvis den er åben.
    fn content_top(&self) -> i32 {
        self.strip_height() + if self.find.is_some() { scale(self.dpi, 76) } else { 0 }
    }

    unsafe fn measure_labels(&mut self) {
        if self.docs.iter().all(|d| d.label_w > 0) {
            return;
        }
        let font = fonts(self.dpi).ui;
        let hdc = GetDC(self.hwnd);
        let previous = SelectObject(hdc, font);
        for d in self.docs.iter_mut().filter(|d| d.label_w == 0) {
            let mut size = SIZE { cx: 0, cy: 0 };
            GetTextExtentPoint32W(hdc, d.label.as_ptr(), d.label.len() as i32, &mut size);
            d.label_w = size.cx.max(1);
        }
        SelectObject(hdc, previous);
        ReleaseDC(self.hwnd, hdc);
    }

    /// Rektangler for hver fane samt plus-knappen. Faner får deres naturlige
    /// bredde, og klemmes ens sammen når de ikke kan være der.
    unsafe fn tab_rects(&mut self) -> (Vec<RECT>, RECT) {
        self.measure_labels();
        let mut rc: RECT = zeroed();
        GetClientRect(self.hwnd, &mut rc);
        let dpi = self.dpi;
        let (top, bottom) = (scale(dpi, 6), self.strip_height());
        let plus_width = scale(dpi, 36);
        // Møtrikken yderst til højre har sin egen plads.
        let available = (rc.right - plus_width - scale(dpi, GEAR_WIDTH)).max(0);
        let chrome = scale(dpi, 12 + 6 + 20 + 8);
        let natural: Vec<i32> = self
            .docs
            .iter()
            .map(|d| (d.label_w + chrome).clamp(scale(dpi, 110), scale(dpi, 280)))
            .collect();
        let squeezed = if natural.iter().sum::<i32>() > available && !natural.is_empty() {
            Some((available / natural.len() as i32).max(scale(dpi, 44)))
        } else {
            None
        };
        let mut x = 0;
        let rects = natural
            .iter()
            .map(|&width| {
                let width = squeezed.unwrap_or(width);
                let r = RECT { left: x, top, right: x + width, bottom };
                x += width;
                r
            })
            .collect();
        (rects, RECT { left: x, top, right: x + plus_width, bottom })
    }

    /// Møtrikken til indstillingerne, helt ude til højre på fanelinjen.
    unsafe fn gear_rect(&self) -> RECT {
        let mut rc: RECT = zeroed();
        GetClientRect(self.hwnd, &mut rc);
        let width = scale(self.dpi, GEAR_WIDTH);
        RECT { left: (rc.right - width).max(0), top: scale(self.dpi, 6), right: rc.right, bottom: self.strip_height() }
    }
}

const GEAR_WIDTH: i32 = 40;

fn close_rect(tab: &RECT, dpi: u32) -> RECT {
    let size = scale(dpi, 20);
    let middle = (tab.top + tab.bottom) / 2;
    let right = tab.right - scale(dpi, 8);
    RECT { left: right - size, top: middle - size / 2, right, bottom: middle - size / 2 + size }
}

fn contains(r: &RECT, x: i32, y: i32) -> bool {
    x >= r.left && x < r.right && y >= r.top && y < r.bottom
}

unsafe fn hit(w: &mut Win, x: i32, y: i32) -> Hit {
    if w.find.is_some() && y >= w.strip_height() && y < w.content_top() {
        let button = find_layout(w).buttons.into_iter().find(|(_, r)| contains(r, x, y));
        return button.map_or(Hit::None, |(button, _)| Hit::Find(button));
    }
    if y < 0 || y >= w.strip_height() {
        return Hit::None;
    }
    let dpi = w.dpi;
    if x >= w.gear_rect().left {
        return Hit::Settings;
    }
    let (rects, plus) = w.tab_rects();
    for (i, r) in rects.iter().enumerate() {
        if x >= r.left && x < r.right {
            return if contains(&close_rect(r, dpi), x, y) { Hit::Close(i) } else { Hit::Tab(i) };
        }
    }
    if x >= plus.left && x < plus.right {
        return Hit::Plus;
    }
    Hit::None
}

unsafe fn fill(hdc: HDC, r: &RECT, color: u32) {
    SetBkColor(hdc, color);
    ExtTextOutW(hdc, 0, 0, ETO_OPAQUE, r, null(), 0, null());
}

unsafe fn glyph(hdc: HDC, font: HFONT, ch: u16, r: &RECT, color: u32) {
    let mut r = *r;
    SelectObject(hdc, font);
    SetTextColor(hdc, color);
    DrawTextW(hdc, &ch, 1, &mut r, DT_SINGLELINE | DT_VCENTER | DT_CENTER | DT_NOPREFIX);
}

unsafe fn paint(hwnd: HWND) {
    let mut ps: PAINTSTRUCT = zeroed();
    let hdc = BeginPaint(hwnd, &mut ps);
    if let Some(w) = win(hwnd) {
        let mut rc: RECT = zeroed();
        GetClientRect(hwnd, &mut rc);
        let (width, height) = (rc.right, w.strip_height());
        if width > 0 && ps.rcPaint.top < height {
            // Fanelinjen tegnes i en bitmap i hukommelsen og kopieres over i ét hug, så den aldrig flimrer.
            let mem = CreateCompatibleDC(hdc);
            let bitmap = CreateCompatibleBitmap(hdc, width, height);
            let old_bitmap = SelectObject(mem, bitmap);
            let old_font = SelectObject(mem, fonts(w.dpi).ui);
            draw_strip(mem, w, width, height);
            BitBlt(hdc, 0, 0, width, height, mem, 0, 0, SRCCOPY);
            SelectObject(mem, old_font);
            SelectObject(mem, old_bitmap);
            DeleteObject(bitmap);
            DeleteDC(mem);
        }
        let bar = w.content_top() - height;
        if bar > 0 && ps.rcPaint.top < height + bar && ps.rcPaint.bottom > height {
            // Søgelinjen, tegnet på samme måde.
            let mem = CreateCompatibleDC(hdc);
            let bitmap = CreateCompatibleBitmap(hdc, width.max(1), bar);
            let old_bitmap = SelectObject(mem, bitmap);
            let old_font = SelectObject(mem, fonts(w.dpi).ui);
            draw_find(mem, w, width, bar);
            BitBlt(hdc, 0, height, width, bar, mem, 0, 0, SRCCOPY);
            SelectObject(mem, old_font);
            SelectObject(mem, old_bitmap);
            DeleteObject(bitmap);
            DeleteDC(mem);
        }
        let height = height + bar;
        let (gutter, below) = (w.gutter, rc.bottom - height);
        if gutter > 0 && below > 0 && ps.rcPaint.left < gutter && ps.rcPaint.bottom > height {
            // Linjenumrene tegnes på samme måde, så de heller ikke flimrer.
            let mem = CreateCompatibleDC(hdc);
            let bitmap = CreateCompatibleBitmap(hdc, gutter, below);
            let old_bitmap = SelectObject(mem, bitmap);
            let old_font = SelectObject(mem, fonts(w.dpi).ui);
            draw_gutter(mem, w, gutter, below);
            BitBlt(hdc, 0, height, gutter, below, mem, 0, 0, SRCCOPY);
            SelectObject(mem, old_font);
            SelectObject(mem, old_bitmap);
            DeleteObject(bitmap);
            DeleteDC(mem);
        }
    }
    EndPaint(hwnd, &ps);
}

unsafe fn draw_strip(hdc: HDC, w: &mut Win, width: i32, height: i32) {
    let theme = app().theme;
    let dpi = w.dpi;
    let f = fonts(dpi);
    let (rects, plus) = w.tab_rects();
    fill(hdc, &RECT { left: 0, top: 0, right: width, bottom: height }, theme.strip);
    SetBkMode(hdc, TRANSPARENT as _);

    for (i, r) in rects.iter().enumerate() {
        let d = &w.docs[i];
        let active = i == w.active;
        let hovered = matches!(w.hover, Hit::Tab(j) | Hit::Close(j) if j == i);
        let close_hot = w.hover == Hit::Close(i);

        if active {
            fill(hdc, r, theme.editor_bg);
            fill(hdc, &RECT { bottom: r.top + scale(dpi, 2), ..*r }, theme.accent);
        } else {
            if hovered {
                fill(hdc, r, theme.hover);
            }
            if i + 1 != w.active {
                let inset = scale(dpi, 9);
                let line = RECT { left: r.right - 1, top: r.top + inset, right: r.right, bottom: r.bottom - inset };
                fill(hdc, &line, theme.sep);
            }
        }

        let close = close_rect(r, dpi);
        let mut text = RECT { left: r.left + scale(dpi, 12), top: r.top, right: close.left - scale(dpi, 6), bottom: r.bottom };
        SelectObject(hdc, f.ui);
        SetTextColor(hdc, if active { theme.text } else { theme.dim });
        DrawTextW(
            hdc,
            d.label.as_ptr(),
            d.label.len() as i32,
            &mut text,
            DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS | DT_NOPREFIX,
        );

        if close_hot {
            fill(hdc, &close, theme.close_hover);
        }
        if d.dirty && !close_hot {
            glyph(hdc, f.ui, GLYPH_DIRTY, &close, if active { theme.accent } else { theme.dim });
        } else if active || hovered {
            glyph(hdc, f.icon, GLYPH_CLOSE, &close, if close_hot { theme.text } else { theme.dim });
        }
    }

    if w.hover == Hit::Plus {
        let inset = scale(dpi, 4);
        let hot = RECT { left: plus.left + inset, top: plus.top + inset, right: plus.right - inset, bottom: plus.bottom - inset };
        fill(hdc, &hot, theme.hover);
    }
    glyph(hdc, f.icon, GLYPH_PLUS, &plus, theme.dim);

    // Møtrikken tegnes sidst og med sin egen baggrund, så sammenklemte faner ikke
    // stikker ind under den.
    let gear = w.gear_rect();
    fill(hdc, &RECT { top: 0, ..gear }, theme.strip);
    let hot = w.hover == Hit::Settings;
    if hot {
        let inset = scale(dpi, 4);
        fill(hdc, &RECT { left: gear.left + inset, top: gear.top + inset, right: gear.right - inset, bottom: gear.bottom - inset }, theme.hover);
    }
    glyph(hdc, f.icon, GLYPH_SETTINGS, &gear, if hot { theme.text } else { theme.dim });

    if let Some(at) = w.drop_hint {
        let x = rects.get(at).map_or(plus.left, |r| r.left);
        let bar = RECT { left: (x - 1).max(0), top: plus.top, right: (x - 1).max(0) + scale(dpi, 3), bottom: height };
        fill(hdc, &bar, theme.accent);
    }
}

// ---------- mus ----------

unsafe fn on_mouse_move(hwnd: HWND, x: i32, y: i32) {
    let Some(w) = win(hwnd) else { return };
    if let Some(drag) = w.drag.as_mut() {
        if !drag.moved {
            let slack_x = GetSystemMetrics(SM_CXDRAG).max(4);
            let slack_y = GetSystemMetrics(SM_CYDRAG).max(4);
            if (x - drag.start.x).abs() < slack_x && (y - drag.start.y).abs() < slack_y {
                return;
            }
            drag.moved = true;
        }
        let (index, grab) = (drag.idx, drag.grab);
        let mut rc: RECT = zeroed();
        GetClientRect(hwnd, &mut rc);
        let margin = scale(w.dpi, 24);
        if y < -margin || y > w.strip_height() + margin || x < -margin || x > rc.right + margin {
            detach(hwnd, index, grab);
            return;
        }
        // Byt først plads, når markøren er forbi midten af nabofanen; ellers
        // hopper faner af forskellig bredde frem og tilbage.
        let (rects, _) = w.tab_rects();
        let middle = |r: &RECT| (r.left + r.right) / 2;
        let target = if index > 0 && x < middle(&rects[index - 1]) {
            index - 1
        } else if index + 1 < rects.len() && x > middle(&rects[index + 1]) {
            index + 1
        } else {
            index
        };
        if target != index {
            w.docs.swap(index, target);
            w.active = target;
            if let Some(drag) = w.drag.as_mut() {
                drag.idx = target;
            }
            invalidate_strip(hwnd);
        }
        return;
    }

    let over = hit(w, x, y);
    if over != w.hover {
        w.hover = over;
        invalidate_strip(hwnd);
    }
    if !w.tracking {
        let mut track = TRACKMOUSEEVENT {
            cbSize: size_of::<TRACKMOUSEEVENT>() as u32,
            dwFlags: TME_LEAVE,
            hwndTrack: hwnd,
            dwHoverTime: 0,
        };
        TrackMouseEvent(&mut track);
        w.tracking = true;
    }
}

unsafe fn on_left_down(hwnd: HWND, x: i32, y: i32, double: bool) {
    let Some(w) = win(hwnd) else { return };
    let over = hit(w, x, y);
    w.press = over;
    match over {
        Hit::Tab(i) => {
            let (rects, _) = w.tab_rects();
            let grab = POINT { x: x - rects[i].left, y };
            activate(hwnd, i);
            if let Some(w) = win(hwnd) {
                w.drag = Some(Drag { idx: i, start: POINT { x, y }, grab, moved: false });
                SetCapture(hwnd);
            }
        }
        Hit::None if double && y < w.strip_height() => new_tab(hwnd),
        _ => {}
    }
}

unsafe fn on_left_up(hwnd: HWND, x: i32, y: i32) {
    let Some(w) = win(hwnd) else { return };
    if w.drag.take().is_some() {
        ReleaseCapture();
    }
    let Some(w) = win(hwnd) else { return };
    let pressed = std::mem::replace(&mut w.press, Hit::None);
    let over = hit(w, x, y);
    if over != pressed {
        return;
    }
    match over {
        Hit::Close(i) => {
            let id = w.docs[i].id;
            close_tab(hwnd, id);
        }
        Hit::Plus => new_tab(hwnd),
        Hit::Find(button) => find_action(hwnd, button),
        Hit::Settings => {
            // Vinduet med indstillinger blokerer, så møtrikken må ikke blive ved med at lyse.
            w.hover = Hit::None;
            invalidate_strip(hwnd);
            command(hwnd, CMD_SETTINGS);
        }
        _ => {}
    }
}

// ---------- træk en fane ud ----------

/// Fanen har forladt fanelinjen. Den får sit eget vindue (eller, hvis den var
/// vinduets eneste fane, flyttes hele vinduet), og Windows' egen vinduesflytning
/// tager over, så snap og skærmkanter virker som normalt.
unsafe fn detach(hwnd: HWND, index: usize, grab: POINT) {
    let Some(w) = win(hwnd) else { return };
    w.drag = None;
    ReleaseCapture();
    let Some(w) = win(hwnd) else { return };
    if index >= w.docs.len() {
        return;
    }

    let floating = if w.docs.len() == 1 {
        hwnd
    } else {
        let mut placement: WINDOWPLACEMENT = zeroed();
        placement.length = size_of::<WINDOWPLACEMENT>() as u32;
        GetWindowPlacement(hwnd, &mut placement);
        let normal = placement.rcNormalPosition;
        let size = RECT { left: 0, top: 0, right: normal.right - normal.left, bottom: normal.bottom - normal.top };

        let moved = remove_doc(hwnd, index);
        let created = create_window(Some(size));
        add_doc(created, moved, None);

        // Placér vinduet, så den udtrukne fane ligger under markøren, præcis hvor der blev taget fat.
        let mut cursor = POINT { x: 0, y: 0 };
        GetCursorPos(&mut cursor);
        let mut frame: RECT = zeroed();
        GetWindowRect(created, &mut frame);
        let mut client_origin = POINT { x: 0, y: 0 };
        ClientToScreen(created, &mut client_origin);
        let (mut grab_x, mut grab_y) = (grab.x, grab.y);
        if let Some(nw) = win(created) {
            let (rects, _) = nw.tab_rects();
            grab_x = grab_x.clamp(0, (rects[0].right - rects[0].left - 1).max(0));
            grab_y = grab_y.clamp(0, nw.strip_height() - 1);
        }
        SetWindowPos(
            created,
            null_mut(),
            cursor.x - (client_origin.x - frame.left) - grab_x,
            cursor.y - (client_origin.y - frame.top) - grab_y,
            0,
            0,
            SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
        );
        show_painted(created, SW_SHOW);
        SetForegroundWindow(created);
        created
    };

    let primary = if GetSystemMetrics(SM_SWAPBUTTON) != 0 { VK_RBUTTON } else { VK_LBUTTON };
    if GetAsyncKeyState(primary as i32) >= 0 {
        // Knappen blev sluppet, mens vinduet blev oprettet; så er der intet at flytte.
        return;
    }
    if let Some(fw) = win(floating) {
        fw.floating = true;
        fw.over_target = false;
    }
    set_alpha(floating, Some(235));
    let mut cursor = POINT { x: 0, y: 0 };
    GetCursorPos(&mut cursor);
    // Starter Windows' egen flytteløkke, som om titellinjen var grebet (samme greb
    // som Chromium bruger til faner). Kaldet blokerer, indtil museknappen slippes.
    SendMessageW(floating, WM_SYSCOMMAND, SC_MOVE as WPARAM | HTCAPTION_HIT, make_lparam(cursor.x, cursor.y));
    finish_float(floating);
}

/// Museknappen er sluppet: flet ind i vinduet under markøren, hvis der er et.
unsafe fn finish_float(floating: HWND) {
    let Some(w) = win(floating) else { return };
    if !w.floating {
        return;
    }
    w.floating = false;
    w.over_target = false;
    set_alpha(floating, None);

    let mut cursor = POINT { x: 0, y: 0 };
    GetCursorPos(&mut cursor);
    let target = find_drop_target(floating, cursor);
    let all: Vec<HWND> = app().wins.iter().map(|w| w.hwnd).collect();
    for h in all {
        if let Some(w) = win(h) {
            if w.drop_hint.take().is_some() {
                invalidate_strip(h);
            }
        }
    }

    let Some((target, at)) = target else { return };
    let Some(w) = win(floating) else { return };
    let moved: Vec<Doc> = std::mem::take(&mut w.docs);
    for (offset, d) in moved.into_iter().enumerate() {
        add_doc(target, d, Some(at + offset));
    }
    DestroyWindow(floating);
    bring_to_front(target);
}

unsafe fn update_drop_hint(floating: HWND) {
    let mut cursor = POINT { x: 0, y: 0 };
    GetCursorPos(&mut cursor);
    let target = find_drop_target(floating, cursor);
    let all: Vec<HWND> = app().wins.iter().map(|w| w.hwnd).collect();
    for h in all {
        let Some(w) = win(h) else { continue };
        let hint = target.filter(|&(t, _)| t == h).map(|(_, at)| at);
        if w.drop_hint != hint {
            w.drop_hint = hint;
            invalidate_strip(h);
        }
    }
    let Some(w) = win(floating) else { return };
    if w.over_target != target.is_some() {
        w.over_target = target.is_some();
        // Gennemsigtigt over et mål, så man kan se hvor fanen lander.
        set_alpha(floating, Some(if target.is_some() { 120 } else { 235 }));
    }
}

/// Det øverste sEdit-vindue under markøren, hvis markøren er over dets fanelinje.
unsafe fn find_drop_target(exclude: HWND, at: POINT) -> Option<(HWND, usize)> {
    let mut h = GetTopWindow(null_mut());
    while !h.is_null() {
        if h != exclude && IsWindowVisible(h) != 0 && IsIconic(h) == 0 {
            if let Some(w) = win(h) {
                let mut frame: RECT = zeroed();
                GetWindowRect(h, &mut frame);
                if PtInRect(&frame, at) != 0 {
                    let mut local = at;
                    ScreenToClient(h, &mut local);
                    let mut rc: RECT = zeroed();
                    GetClientRect(h, &mut rc);
                    if local.y < 0 || local.y >= w.strip_height() || local.x < 0 || local.x >= rc.right {
                        return None;
                    }
                    let (rects, _) = w.tab_rects();
                    let index = rects.iter().filter(|r| (r.left + r.right) / 2 < local.x).count();
                    return Some((h, index));
                }
            }
        }
        h = GetWindow(h, GW_HWNDNEXT);
    }
    None
}

unsafe fn set_alpha(hwnd: HWND, alpha: Option<u8>) {
    let style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
    let layered = WS_EX_LAYERED as isize;
    match alpha {
        Some(value) => {
            if style & layered == 0 {
                SetWindowLongPtrW(hwnd, GWL_EXSTYLE, style | layered);
            }
            SetLayeredWindowAttributes(hwnd, 0, value, LWA_ALPHA);
        }
        None if style & layered != 0 => {
            SetWindowLongPtrW(hwnd, GWL_EXSTYLE, style & !layered);
            RedrawWindow(hwnd, null(), null_mut(), RDW_INVALIDATE | RDW_ERASE | RDW_FRAME | RDW_ALLCHILDREN);
        }
        None => {}
    }
}

// ---------- dokumenter ----------

fn key_of(path: &Path) -> String {
    path.to_string_lossy().to_lowercase()
}

fn name_of(path: &Path) -> String {
    path.file_name().map_or_else(|| path.display().to_string(), |n| n.to_string_lossy().into_owned())
}

/// Opretter et dokument med et tomt tekstfelt i vinduet. `untitled_name` genbruger
/// navnet på en gendannet fane uden fil; ellers får en ny sådan fane næste nummer.
unsafe fn new_doc(hwnd: HWND, path: Option<PathBuf>, untitled_name: Option<&str>) -> Doc {
    let a = app();
    a.next_id += 1;
    let id = a.next_id;
    let dpi = win(hwnd).map_or(96, |w| w.dpi);
    let edit = doc::create_edit(hwnd, fonts(dpi).edit, &a.theme, a.settings.wrap);
    let (name, key) = match (&path, untitled_name) {
        (Some(p), _) => (name_of(p), key_of(p)),
        (None, Some(name)) => {
            // Nye faner må ikke få et nummer, der allerede er i brug af en gendannet.
            let number = name.rsplit(' ').next().and_then(|n| n.parse::<u32>().ok()).unwrap_or(0);
            a.untitled = a.untitled.max(number);
            (name.to_string(), String::new())
        }
        (None, None) => {
            a.untitled += 1;
            (format!("{} {}", lang::s().untitled, a.untitled), String::new())
        }
    };
    Doc {
        id,
        path,
        key,
        name,
        label: Vec::new(),
        label_w: 0,
        edit,
        dpi,
        enc: Enc::Utf8,
        eol: Eol::Crlf,
        dirty: false,
        stamp: None,
        saved: doc::signature(&[]),
    }
}

/// Fanetekster. To faner med samme filnavn (fx to `.env`) får mappenavnet foran.
unsafe fn refresh_labels(hwnd: HWND) {
    let Some(w) = win(hwnd) else { return };
    let names: Vec<String> = w.docs.iter().map(|d| d.name.to_lowercase()).collect();
    for (i, d) in w.docs.iter_mut().enumerate() {
        let clash = names.iter().enumerate().any(|(j, n)| j != i && *n == names[i]);
        let parent = d.path.as_deref().and_then(Path::parent).and_then(Path::file_name);
        let label = match parent {
            Some(dir) if clash => format!("{}\\{}", dir.to_string_lossy(), d.name),
            _ => d.name.clone(),
        };
        let label: Vec<u16> = label.encode_utf16().collect();
        if label != d.label {
            d.label = label;
            d.label_w = 0;
        }
    }
    invalidate_strip(hwnd);
}

/// Sætter et dokument ind i et vindue (og flytter dets tekstfelt med) og gør det aktivt.
unsafe fn add_doc(hwnd: HWND, mut d: Doc, at: Option<usize>) {
    let Some(w) = win(hwnd) else { return };
    if GetParent(d.edit) != hwnd {
        ShowWindow(d.edit, SW_HIDE);
        SetParent(d.edit, hwnd);
    }
    if d.dpi != w.dpi {
        doc::style_edit(d.edit, fonts(w.dpi).edit, &app().theme);
        d.dpi = w.dpi;
        d.label_w = 0;
    }
    highlight::set_lang(d.edit, lang_for(d.path.as_deref()));
    let Some(w) = win(hwnd) else { return };
    // Prikken for ugemte ændringer holdes ellers kun ved lige for den aktive fane, så en
    // gendannet kladde ville stå uden prik, indtil man klikkede på den.
    d.refresh_dirty(true);
    let index = at.unwrap_or(w.docs.len()).min(w.docs.len());
    w.docs.insert(index, d);
    w.active = index;
    refresh_labels(hwnd);
    show_active(hwnd);
    sync_watch();
}

/// Tager et dokument ud af vinduet. Kalderen ejer derefter tekstfeltet.
unsafe fn remove_doc(hwnd: HWND, index: usize) -> Doc {
    let w = win(hwnd).expect("vinduet findes");
    let removed = w.docs.remove(index);
    if index < w.active {
        w.active -= 1;
    }
    w.active = w.active.min(w.docs.len().saturating_sub(1));
    ShowWindow(removed.edit, SW_HIDE);
    refresh_labels(hwnd);
    show_active(hwnd);
    removed
}

unsafe fn new_tab(hwnd: HWND) {
    let d = new_doc(hwnd, None, None);
    add_doc(hwnd, d, None);
}

fn find_open(path: &Path) -> Option<(HWND, usize)> {
    let key = key_of(path);
    for w in app().wins.iter() {
        if let Some(i) = w.docs.iter().position(|d| !d.key.is_empty() && d.key == key) {
            return Some((w.hwnd, i));
        }
    }
    None
}

/// Åbner hver sti som en fane i vinduet. Filer der allerede er åbne får fokus i stedet.
unsafe fn open_paths(hwnd: HWND, paths: &[PathBuf]) {
    for path in paths {
        if let Some((owner, index)) = find_open(path) {
            activate(owner, index);
            if owner != hwnd {
                bring_to_front(owner);
            }
            continue;
        }
        let Some(w) = win(hwnd) else { return };
        // En tom, urørt "Unavngivet"-fane erstattes af filen, som i Notepad++.
        let pristine = match w.docs.as_slice() {
            [only] if only.path.is_none() && !doc::is_modified(only.edit) && doc::text_len(only.edit) == 0 => Some(only.id),
            _ => None,
        };

        // Blev filen lukket sammen med et vindue tidligere i denne kørsel, hentes den
        // frem igen med de ugemte ændringer, den havde.
        let key = key_of(path);
        let parked = app().parked.iter().position(|s| s.path.as_deref().map(key_of).as_deref() == Some(key.as_str()));
        if let Some(position) = parked {
            let snapshot = app().parked.remove(position);
            restore_snapshot(hwnd, snapshot);
            resolve_conflicts();
        } else {
            let exists = path.exists();
            let mut d = new_doc(hwnd, Some(path.clone()), None);
            if exists {
                if let Err(error) = d.load() {
                    DestroyWindow(d.edit);
                    message(hwnd, &format!("{}\n\n{error}", lang::fill(lang::s().could_not_open, &[&path.display().to_string()])), MB_OK | MB_ICONERROR);
                    continue;
                }
            }
            add_doc(hwnd, d, None);
        }

        if let Some((w, index)) = pristine.and_then(find_doc) {
            let owner = w.hwnd;
            let old = remove_doc(owner, index);
            DestroyWindow(old.edit);
        }
    }
}

// ---------- session: husk fanerne og den ugemte tekst ----------

/// Det der skal til for at genskabe fanen senere. None for en tom fane uden fil.
unsafe fn snapshot(d: &mut Doc, active: bool) -> Option<Snapshot> {
    let dirty = d.refresh_dirty(true);
    if d.path.is_none() && !dirty {
        return None;
    }
    Some(Snapshot {
        path: d.path.clone(),
        name: d.name.clone(),
        enc: d.enc,
        eol: d.eol,
        draft: dirty.then(|| doc::get_text(d.edit)),
        base: d.saved,
        selection: doc::selection(d.edit),
        active,
    })
}

unsafe fn snapshot_window(hwnd: HWND) -> Vec<Snapshot> {
    let Some(w) = win(hwnd) else { return Vec::new() };
    let active = w.active;
    w.docs.iter_mut().enumerate().filter_map(|(i, d)| snapshot(d, i == active)).collect()
}

/// Lukker vinduet uden at spørge: fanerne og deres ugemte tekst huskes i stedet.
/// Er det appens sidste vindue, skrives sessionen til disken, før vinduet forsvinder.
unsafe fn close_window(hwnd: HWND) {
    let tabs = snapshot_window(hwnd);
    let a = app();
    let last = a.wins.len() <= 1;
    let remembered = a.parked.len();
    for earlier in a.parked.iter_mut() {
        earlier.active = false;
    }
    a.parked.extend(tabs);

    if last {
        if let Err(error) = session::save(&a.session_dir, &a.parked) {
            // Kan teksten ikke huskes, må den ikke bare forsvinde: spørg som i gamle dage.
            a.parked.truncate(remembered);
            message(
                hwnd,
                &format!("{}\n\n{error}", lang::s().session_not_saved),
                MB_OK | MB_ICONERROR,
            );
            if !confirm_close_all(hwnd) {
                return;
            }
        }
        app().session_written = true;
    }
    DestroyWindow(hwnd);
}

/// Skriver alle åbne vinduers faner til sessionen uden at lukke noget.
unsafe fn save_all_windows() {
    let windows: Vec<HWND> = app().wins.iter().map(|w| w.hwnd).collect();
    let mut tabs = app().parked.clone();
    for earlier in tabs.iter_mut() {
        earlier.active = false;
    }
    for hwnd in windows {
        tabs.extend(snapshot_window(hwnd));
    }
    let _ = session::save(&app().session_dir, &tabs);
}

/// Åbner fanerne fra sidste gang sEdit blev lukket.
unsafe fn restore_session(hwnd: HWND) {
    let mut active = None;
    for tab in session::load(&app().session_dir) {
        let was_active = tab.active;
        if let Some(id) = restore_snapshot(hwnd, tab) {
            if was_active {
                active = Some(id);
            }
        }
    }
    if let Some((w, index)) = active.and_then(find_doc) {
        let owner = w.hwnd;
        activate(owner, index);
    }
}

/// Genskaber én fane. Filen læses altid først, så fanen bygger på det der ligger på
/// disken nu; en kladde lægges ovenpå som ét trin, der kan fortrydes med Ctrl+Z.
unsafe fn restore_snapshot(hwnd: HWND, tab: Snapshot) -> Option<u32> {
    let Some(path) = tab.path.clone() else {
        let draft = tab.draft?;
        let mut d = new_doc(hwnd, None, Some(&tab.name));
        d.enc = tab.enc;
        d.eol = tab.eol;
        doc::replace_undoable(d.edit, &draft, tab.selection);
        let id = d.id;
        add_doc(hwnd, d, None);
        return Some(id);
    };
    if find_open(&path).is_some() {
        return None;
    }

    let mut d = new_doc(hwnd, Some(path.clone()), None);
    let loaded = path.exists() && d.load().is_ok();
    let id = d.id;
    match tab.draft {
        // Ingen ugemte ændringer: fanen viser bare filen, som den ser ud nu.
        None if loaded => doc::replace_selection(d.edit, tab.selection),
        None => {
            // Filen er væk, og der var intet ugemt at redde.
            DestroyWindow(d.edit);
            return None;
        }
        Some(draft) if !loaded => {
            // Filen er væk eller kan ikke læses; kladden er alt hvad der er tilbage.
            d.enc = tab.enc;
            d.eol = tab.eol;
            doc::replace_undoable(d.edit, &draft, tab.selection);
        }
        Some(draft) if d.saved == tab.base => doc::replace_undoable(d.edit, &draft, tab.selection),
        // Filen er ændret, siden kladden blev skrevet. Fanen viser filens nye indhold,
        // indtil brugeren har valgt i `resolve_conflicts`.
        Some(draft) => app().conflicts.push(Conflict { id, draft, selection: tab.selection }),
    }
    add_doc(hwnd, d, None);
    Some(id)
}

/// Spørger for hver gendannet fane, hvor filen er ændret i mellemtiden: skal filens
/// nye indhold bruges, skal det ugemte gemmes oven i, eller skal det ugemte bare vises?
unsafe fn resolve_conflicts() {
    const RELOAD: i32 = 1;
    const SAVE_MINE: i32 = 2;
    const KEEP_MINE: i32 = 3;
    while !app().conflicts.is_empty() {
        let conflict = app().conflicts.remove(0);
        let Some((w, index)) = find_doc(conflict.id) else { continue };
        let hwnd = w.hwnd;
        let name = w.docs[index].name.clone();
        activate(hwnd, index);
        let words = lang::s();
        // Forklaringen nævner knapperne ved de navne, der står på dem.
        let buttons = [words.reload, words.save_mine, words.keep_mine].map(lang::plain);
        let answer = dialog::choose(
            hwnd,
            &lang::fill(words.conflict, &[&name, &buttons[0], &buttons[1], &buttons[2]]),
            &[
                Choice { id: RELOAD, label: words.reload },
                Choice { id: SAVE_MINE, label: words.save_mine },
                Choice { id: KEEP_MINE, label: words.keep_mine },
            ],
            2,
            Some(KEEP_MINE),
        );
        if answer != RELOAD {
            let Some((w, index)) = find_doc(conflict.id) else { continue };
            doc::replace_undoable(w.docs[index].edit, &conflict.draft, conflict.selection);
            if answer == SAVE_MINE {
                save_doc(hwnd, conflict.id, false);
            }
        }
        sync_dirty(hwnd);
    }
}

unsafe fn message(hwnd: HWND, text: &str, flags: u32) -> i32 {
    crate::dialog::show(hwnd, text, flags)
}

/// Gemmer dokumentet. Spørger om et filnavn, hvis det ikke har et, eller ved Gem som.
unsafe fn save_doc(hwnd: HWND, id: u32, ask_name: bool) -> bool {
    let Some((w, index)) = find_doc(id) else { return false };
    if ask_name || w.docs[index].path.is_none() {
        let current = w.docs[index].path.clone();
        let Some(chosen) = save_dialog(hwnd, current.as_deref()) else { return false };
        let Some((w, index)) = find_doc(id) else { return false };
        let d = &mut w.docs[index];
        d.name = name_of(&chosen);
        d.key = key_of(&chosen);
        // Et nyt filnavn kan betyde et andet sprog (eller intet).
        highlight::set_lang(d.edit, lang_for(Some(&chosen)));
        d.path = Some(chosen);
        d.stamp = None;
    }
    let Some((w, index)) = find_doc(id) else { return false };
    let owner = w.hwnd;
    let result = w.docs[index].save();
    refresh_labels(owner);
    update_title(owner);
    sync_watch();
    match result {
        Ok(()) => true,
        Err(error) => {
            message(owner, &format!("{}\n\n{error}", lang::s().could_not_save), MB_OK | MB_ICONERROR);
            false
        }
    }
}

/// Spørger om ugemte ændringer skal gemmes. Returnerer false, hvis brugeren fortryder.
unsafe fn confirm_discard(hwnd: HWND, id: u32) -> bool {
    let Some((w, index)) = find_doc(id) else { return true };
    if !w.docs[index].refresh_dirty(true) {
        return true;
    }
    let name = w.docs[index].name.clone();
    activate(hwnd, index);
    // Knapperne siger hvad de gør, så man ikke skal læse spørgsmålet for at vælge rigtigt.
    let words = lang::s();
    let choices = [
        Choice { id: IDYES, label: words.save_it },
        Choice { id: IDNO, label: words.dont_save },
        Choice { id: IDCANCEL, label: words.cancel },
    ];
    match dialog::choose(hwnd, &lang::fill(words.save_changes, &[&name]), &choices, 0, Some(IDCANCEL)) {
        IDYES => save_doc(hwnd, id, false),
        IDNO => true,
        _ => false,
    }
}

unsafe fn close_tab(hwnd: HWND, id: u32) -> bool {
    if !confirm_discard(hwnd, id) {
        return false;
    }
    let Some((w, index)) = find_doc(id) else { return true };
    let owner = w.hwnd;
    let closed = remove_doc(owner, index);
    DestroyWindow(closed.edit);
    if win(owner).is_some_and(|w| w.docs.is_empty()) {
        DestroyWindow(owner);
    } else {
        sync_watch();
    }
    true
}

unsafe fn confirm_close_all(hwnd: HWND) -> bool {
    let Some(w) = win(hwnd) else { return true };
    let ids: Vec<u32> = w.docs.iter().map(|d| d.id).collect();
    ids.into_iter().all(|id| confirm_discard(hwnd, id))
}

/// Holder fanens "ugemt"-prik i takt med tekstfeltets ændringsflag.
unsafe fn sync_dirty(hwnd: HWND) {
    let Some(w) = win(hwnd) else { return };
    let Some(d) = w.docs.get_mut(w.active) else { return };
    let was = d.dirty;
    if d.refresh_dirty(false) != was {
        invalidate_strip(hwnd);
    }
}

pub unsafe fn on_drop_files(hwnd: HWND, drop: HDROP) {
    let count = DragQueryFileW(drop, u32::MAX, null_mut(), 0);
    let mut paths = Vec::with_capacity(count as usize);
    for i in 0..count {
        let len = DragQueryFileW(drop, i, null_mut(), 0) as usize;
        let mut buf = vec![0u16; len + 1];
        DragQueryFileW(drop, i, buf.as_mut_ptr(), buf.len() as u32);
        paths.push(PathBuf::from(OsString::from_wide(&buf[..len])));
    }
    DragFinish(drop);
    open_paths(hwnd, &paths);
    bring_to_front(hwnd);
}

// ---------- fildialoger ----------

fn dialog_filter() -> Vec<u16> {
    format!("{}\0*.*\0\0", lang::s().all_files).encode_utf16().collect()
}

unsafe fn initial_dir(hwnd: HWND) -> Option<Vec<u16>> {
    let w = win(hwnd)?;
    let dir = w.docs.get(w.active)?.path.as_deref()?.parent()?;
    Some(wide_os(dir.as_os_str()))
}

unsafe fn open_dialog(hwnd: HWND) -> Vec<PathBuf> {
    let filter = dialog_filter();
    let dir = initial_dir(hwnd);
    let mut buf = vec![0u16; 32768];
    let mut ofn: OPENFILENAMEW = zeroed();
    ofn.lStructSize = size_of::<OPENFILENAMEW>() as u32;
    ofn.hwndOwner = hwnd;
    ofn.lpstrFilter = filter.as_ptr();
    ofn.lpstrFile = buf.as_mut_ptr();
    ofn.nMaxFile = buf.len() as u32;
    ofn.lpstrInitialDir = dir.as_ref().map_or(null(), |d| d.as_ptr());
    ofn.Flags = OFN_EXPLORER | OFN_ALLOWMULTISELECT | OFN_FILEMUSTEXIST | OFN_HIDEREADONLY | OFN_NOCHANGEDIR;
    if GetOpenFileNameW(&mut ofn) == 0 {
        return Vec::new();
    }
    // Én fil: hele stien. Flere filer: mappen efterfulgt af filnavnene, adskilt af NUL.
    let parts: Vec<OsString> = buf
        .split(|&c| c == 0)
        .take_while(|part| !part.is_empty())
        .map(OsString::from_wide)
        .collect();
    match parts.as_slice() {
        [] => Vec::new(),
        [single] => vec![PathBuf::from(single)],
        [dir, names @ ..] => names.iter().map(|n| Path::new(dir).join(n)).collect(),
    }
}

unsafe fn save_dialog(hwnd: HWND, current: Option<&Path>) -> Option<PathBuf> {
    let filter = dialog_filter();
    let dir = match current.and_then(Path::parent) {
        Some(parent) => Some(wide_os(parent.as_os_str())),
        None => initial_dir(hwnd),
    };
    let mut buf = vec![0u16; 32768];
    if let Some(name) = current.and_then(Path::file_name) {
        let name: Vec<u16> = wide_os(name);
        buf[..name.len()].copy_from_slice(&name);
    }
    let mut ofn: OPENFILENAMEW = zeroed();
    ofn.lStructSize = size_of::<OPENFILENAMEW>() as u32;
    ofn.hwndOwner = hwnd;
    ofn.lpstrFilter = filter.as_ptr();
    ofn.lpstrFile = buf.as_mut_ptr();
    ofn.nMaxFile = buf.len() as u32;
    ofn.lpstrInitialDir = dir.as_ref().map_or(null(), |d| d.as_ptr());
    ofn.Flags = OFN_EXPLORER | OFN_OVERWRITEPROMPT | OFN_HIDEREADONLY | OFN_NOCHANGEDIR;
    if GetSaveFileNameW(&mut ofn) == 0 {
        return None;
    }
    let len = buf.iter().position(|&c| c == 0).unwrap_or(0);
    (len > 0).then(|| PathBuf::from(OsString::from_wide(&buf[..len])))
}

// ---------- filer ændret af andre programmer ----------

unsafe fn sync_watch() {
    let mut wanted: Vec<(String, PathBuf)> = Vec::new();
    for w in app().wins.iter() {
        for d in w.docs.iter() {
            let Some(dir) = d.path.as_deref().and_then(Path::parent) else { continue };
            let key = key_of(dir);
            if !wanted.iter().any(|(k, _)| *k == key) {
                wanted.push((key, dir.to_path_buf()));
            }
        }
    }
    app().watch.sync(&wanted);
}

/// Planlægger et filtjek om lidt. Flere signaler i træk giver stadig kun ét tjek.
unsafe fn arm_check(delay_ms: u32) {
    let a = app();
    if !a.timer_armed && !a.ipc.is_null() {
        a.timer_armed = true;
        SetTimer(a.ipc, TIMER_CHECK, delay_ms, None);
    }
}

/// Genindlæser de åbne filer, som er ændret på disken siden vi sidst læste eller skrev dem.
unsafe fn check_files() {
    let a = app();
    if a.checking {
        a.recheck = true;
        return;
    }
    a.checking = true;

    let mut open: Vec<(HWND, u32, PathBuf, Option<doc::Stamp>)> = Vec::new();
    for w in a.wins.iter() {
        for d in w.docs.iter() {
            if let Some(path) = &d.path {
                open.push((w.hwnd, d.id, path.clone(), d.stamp));
            }
        }
    }

    let mut retry = false;
    for (hwnd, id, path, known) in open {
        // En fil der er væk lader vi være; teksten bliver stående i fanen.
        let Some(now) = doc::stat(&path) else { continue };
        if Some(now) == known {
            continue;
        }
        let Some((w, index)) = find_doc(id) else { continue };
        if !w.docs[index].refresh_dirty(true) {
            // Programmet der skriver kan stadig have filen låst; så prøver vi igen om lidt.
            if w.docs[index].load().is_err() {
                retry = true;
            }
            invalidate_strip(hwnd);
        } else if GetForegroundWindow() == hwnd {
            // Ugemte ændringer overskrives aldrig uden at spørge. Stemplet opdateres
            // først, så der kun spørges én gang pr. ændring.
            w.docs[index].stamp = Some(now);
            let name = w.docs[index].name.clone();
            activate(hwnd, index);
            // "Behold mit" er valgt fra start og er også svaret på Esc, så et hurtigt
            // tastetryk aldrig kasserer ugemte ændringer.
            let words = lang::s();
            let choices = [Choice { id: IDYES, label: words.reload }, Choice { id: IDNO, label: words.keep_mine }];
            let answer = dialog::choose(hwnd, &lang::fill(words.changed_outside, &[&name]), &choices, 1, Some(IDNO));
            if answer == IDYES {
                if let Some((w, index)) = find_doc(id) {
                    if w.docs[index].load().is_err() {
                        w.docs[index].stamp = known;
                        retry = true;
                    }
                    invalidate_strip(w.hwnd);
                }
            }
        }
        // Er vinduet i baggrunden, spørges der når det får fokus igen.
    }

    let a = app();
    a.checking = false;
    if retry && a.retries < 40 {
        a.retries += 1;
        arm_check(100);
    } else {
        a.retries = 0;
        if std::mem::take(&mut a.recheck) {
            arm_check(50);
        }
    }
}

// ---------- én kørende instans ----------

unsafe extern "system" fn ipc_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_COPYDATA => {
            let data = &*(lp as *const COPYDATASTRUCT);
            if data.dwData != IPC_MAGIC {
                return 0;
            }
            if data.cbData >= 2 && !data.lpData.is_null() {
                let units = std::slice::from_raw_parts(data.lpData as *const u16, data.cbData as usize / 2);
                let paths = units
                    .split(|&c| c == b'\n' as u16)
                    .filter(|part| !part.is_empty())
                    .map(|part| PathBuf::from(OsString::from_wide(part)));
                app().pending.extend(paths);
            }
            // Selve åbningen udskydes, så den afsendende proces ikke hænger på en dialog hos os.
            PostMessageW(hwnd, WM_APP_OPEN, 0, 0);
            1
        }
        WM_APP_OPEN => {
            let a = app();
            let paths = std::mem::take(&mut a.pending);
            let target = match win(a.last_active) {
                Some(w) => w.hwnd,
                None => match a.wins.first() {
                    Some(w) => w.hwnd,
                    None => return 0,
                },
            };
            open_paths(target, &paths);
            let front = paths.last().and_then(|p| find_open(p)).map_or(target, |(owner, _)| owner);
            bring_to_front(front);
            0
        }
        WM_TIMER => {
            KillTimer(hwnd, TIMER_CHECK);
            app().timer_armed = false;
            check_files();
            0
        }
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}

// ---------- søg og erstat ----------

const GLYPH_UP: u16 = 0xE70E;
const GLYPH_DOWN: u16 = 0xE70D;
const EN_CHANGE: usize = 0x0300;
const EM_SETSEL: u32 = 0x00B1;
const EM_SCROLLCARET: u32 = 0x00B7;
const EM_REPLACESEL: u32 = 0x00C2;

#[derive(Clone, Copy, PartialEq)]
enum FindButton {
    Prev,
    Next,
    Case,
    Regex,
    Word,
    Close,
    Replace,
    ReplaceAll,
}

/// Søgelinjen under fanelinjen. Den hører til vinduet og søger i den aktive fane.
struct FindBar {
    find: HWND,
    replace: HWND,
    match_case: bool,
    /// Om søgeteksten er et regulært udtryk.
    regex: bool,
    /// "3 af 12", "Ingen fundet" eller "5 erstattet".
    status: String,
}

/// Hvor søgelinjens dele står, i vinduets koordinater.
struct FindLayout {
    find: RECT,
    replace: RECT,
    status: RECT,
    buttons: Vec<(FindButton, RECT)>,
}

unsafe fn find_layout(w: &Win) -> FindLayout {
    let dpi = w.dpi;
    let mut rc: RECT = zeroed();
    GetClientRect(w.hwnd, &mut rc);
    let top = w.strip_height();
    let s = |v: i32| scale(dpi, v);
    let (row, gap) = (s(28), s(4));
    let (first, second) = (top + s(8), top + s(8) + row + gap);
    // Teksterne er ikke lige lange på alle sprog, så de får den plads de fylder,
    // dog mindst den de har på dansk.
    let text = find_text_widths(dpi);
    let left = s(72).max(s(12) + text.label + s(10));
    let status = s(92).max(text.status + s(8));
    let word = s(44).max(text.word + s(16));
    let replace = s(84).max(text.replace + s(24));
    let replace_all = s(104).max(text.replace_all + s(24));
    // Det der står til højre for søgefeltet: status, pile, kontakter og lukkeknap.
    let prev = status + s(4);
    let word_at = prev + s(148);
    let tail = word_at + word + s(4) + s(28);
    let field = (rc.right - left - s(8) - tail - s(20)).clamp(s(120), s(320));
    let after = left + field + s(8);
    let at = |x: i32, y: i32, width: i32| RECT { left: x, top: y, right: x + width, bottom: y + row };
    FindLayout {
        find: at(left, first, field),
        replace: at(left, second, field),
        status: at(after, first, status),
        buttons: vec![
            (FindButton::Prev, at(after + prev, first, s(28))),
            (FindButton::Next, at(after + prev + s(32), first, s(28))),
            (FindButton::Case, at(after + prev + s(68), first, s(36))),
            (FindButton::Regex, at(after + prev + s(108), first, s(36))),
            (FindButton::Word, at(after + word_at, first, word)),
            (FindButton::Close, at((rc.right - s(40)).max(after + word_at + word + s(4)), first, s(28))),
            (FindButton::Replace, at(after, second, replace)),
            (FindButton::ReplaceAll, at(after + replace + s(8), second, replace_all)),
        ],
    }
}

/// Bredderne på søgelinjens tekster i brugerfladens skrift.
#[derive(Clone, Copy)]
struct FindText {
    /// Den bredeste af de to tekster foran felterne.
    label: i32,
    /// Den bredeste besked ved siden af søgefeltet.
    status: i32,
    word: i32,
    replace: i32,
    replace_all: i32,
}

/// Måler søgelinjens tekster. Resultatet huskes, til sproget eller skærmskaleringen
/// skifter, fordi layoutet beregnes ved hver musebevægelse.
unsafe fn find_text_widths(dpi: u32) -> FindText {
    static mut CACHE: Option<(u32, usize, FindText)> = None;
    let key = (dpi, lang::current());
    if let Some((cached_dpi, cached_lang, text)) = *std::ptr::addr_of!(CACHE) {
        if (cached_dpi, cached_lang) == key {
            return text;
        }
    }
    let words = lang::s();
    let hdc = GetDC(null_mut());
    let previous = SelectObject(hdc, fonts(dpi).ui);
    let width = |s: &str| {
        let text: Vec<u16> = s.encode_utf16().collect();
        let mut size = SIZE { cx: 0, cy: 0 };
        GetTextExtentPoint32W(hdc, text.as_ptr(), text.len() as i32, &mut size);
        size.cx
    };
    let text = FindText {
        label: width(words.find).max(width(words.replace)),
        status: width(words.none_found).max(width(words.invalid_regex)).max(width(&lang::fill(words.of_count, &["999", "999"]))),
        word: width(words.word),
        replace: width(words.replace),
        replace_all: width(words.replace_all),
    };
    SelectObject(hdc, previous);
    ReleaseDC(null_mut(), hdc);
    *std::ptr::addr_of_mut!(CACHE) = Some((key.0, key.1, text));
    text
}

/// Lægger de to tekstfelter på plads i deres bokse og giver dem tema og skrift.
unsafe fn place_find_inputs(hwnd: HWND) {
    let Some(w) = win(hwnd) else { return };
    let Some(bar) = w.find.as_ref() else { return };
    let (find, replace) = (bar.find, bar.replace);
    let layout = find_layout(w);
    let (dpi, theme) = (w.dpi, app().theme);
    let font = fonts(dpi).ui;
    for (input, field) in [(find, layout.find), (replace, layout.replace)] {
        doc::style_input(input, font, theme.hover, theme.text);
        // Feltet er en linje højt og står midt i sin boks med lidt luft i siderne.
        let height = scale(dpi, 18);
        let inset = scale(dpi, 6);
        let y = field.top + (field.bottom - field.top - height) / 2;
        SetWindowPos(input, null_mut(), field.left + inset, y, field.right - field.left - 2 * inset, height, SWP_NOZORDER | SWP_NOACTIVATE);
    }
}

/// Åbner søgelinjen, eller flytter fokus til den, hvis den allerede er åben. Er der
/// markeret et ord eller et stykke af en linje, bliver det til søgeteksten.
unsafe fn find_show(hwnd: HWND) {
    let Some(w) = win(hwnd) else { return };
    let Some(edit) = w.docs.get(w.active).map(|d| d.edit) else { return };
    if w.find.is_none() {
        let theme = app().theme;
        let font = fonts(w.dpi).ui;
        let find = doc::create_input(hwnd, font, theme.hover, theme.text);
        let replace = doc::create_input(hwnd, font, theme.hover, theme.text);
        let Some(w) = win(hwnd) else { return };
        w.find = Some(FindBar { find, replace, match_case: false, regex: false, status: String::new() });
        layout(hwnd);
    }
    let Some(bar) = win(hwnd).and_then(|w| w.find.as_ref()) else { return };
    let find = bar.find;
    let (start, end) = doc::selection(edit);
    if end > start && end - start <= 200 {
        let text = doc::get_text(edit);
        let chosen = text.get(start as usize..end as usize).unwrap_or(&[]);
        if !chosen.is_empty() && !chosen.contains(&13) {
            let mut terminated = chosen.to_vec();
            terminated.push(0);
            SendMessageW(find, WM_SETTEXT, 0, terminated.as_ptr() as LPARAM);
        }
    }
    SetFocus(find);
    SendMessageW(find, EM_SETSEL, 0, -1);
    find_step(hwnd, 0);
}

unsafe fn find_close(hwnd: HWND) {
    let Some(w) = win(hwnd) else { return };
    let Some(bar) = w.find.take() else { return };
    let edit = w.docs.get(w.active).map(|d| d.edit);
    DestroyWindow(bar.find);
    DestroyWindow(bar.replace);
    layout(hwnd);
    if let Some(edit) = edit {
        SetFocus(edit);
    }
}

/// Det søgelinjen arbejder med lige nu.
struct FindInputs {
    /// Den aktive fanes tekstfelt.
    edit: HWND,
    needle: Vec<u16>,
    /// Det der står i erstatfeltet.
    with: Vec<u16>,
    match_case: bool,
    regex: bool,
    whole_word: bool,
}

impl FindInputs {
    fn query(&self) -> find::Query<'_> {
        find::Query { needle: &self.needle, match_case: self.match_case, regex: self.regex, whole_word: self.whole_word }
    }
}

unsafe fn find_inputs(hwnd: HWND) -> Option<FindInputs> {
    let w = win(hwnd)?;
    let bar = w.find.as_ref()?;
    let edit = w.docs.get(w.active)?.edit;
    Some(FindInputs {
        edit,
        needle: doc::get_text(bar.find),
        with: doc::get_text(bar.replace),
        match_case: bar.match_case,
        regex: bar.regex,
        // "Kun hele ord" er en indstilling, så valget huskes til næste gang.
        whole_word: app().settings.whole_word,
    })
}

unsafe fn set_find_status(hwnd: HWND, status: String) {
    if let Some(bar) = win(hwnd).and_then(|w| w.find.as_mut()) {
        bar.status = status;
    }
    invalidate_strip(hwnd);
}

/// Går til næste (1) eller forrige (-1) fund, eller til fundet ved markøren (0).
unsafe fn find_step(hwnd: HWND, direction: i32) {
    let Some(inputs) = find_inputs(hwnd) else { return };
    if inputs.needle.is_empty() {
        set_find_status(hwnd, String::new());
        return;
    }
    let text = doc::get_text(inputs.edit);
    let found = match find::matches(&text, &inputs.query()) {
        Ok(found) => found,
        // Et regulært udtryk der ikke er skrevet færdigt endnu.
        Err(problem) => return set_find_status(hwnd, problem),
    };
    let (start, end) = doc::selection(inputs.edit);
    let status = match find::pick(&found, start.max(0) as usize, end.max(0) as usize, direction) {
        Some(index) => {
            let (from, to) = found[index];
            doc::replace_selection(inputs.edit, (from as i32, to as i32));
            SendMessageW(inputs.edit, EM_SCROLLCARET, 0, 0);
            lang::fill(lang::s().of_count, &[&(index + 1).to_string(), &found.len().to_string()])
        }
        None => lang::s().none_found.to_string(),
    };
    set_find_status(hwnd, status);
}

/// Erstatter fundet der er markeret, og går til det næste. Er intet fund markeret
/// endnu, går den bare til det første.
unsafe fn find_replace(hwnd: HWND) {
    let Some(inputs) = find_inputs(hwnd) else { return };
    if inputs.needle.is_empty() {
        return;
    }
    let text = doc::get_text(inputs.edit);
    let (start, end) = doc::selection(inputs.edit);
    let range = (start.max(0) as usize, end.max(0) as usize);
    match find::replacement(&text, &inputs.query(), range, &inputs.with) {
        Ok(Some(mut with)) => {
            with.push(0);
            // Som én rettelse, der kan fortrydes med Ctrl+Z.
            SendMessageW(inputs.edit, EM_REPLACESEL, 1, with.as_ptr() as LPARAM);
            find_step(hwnd, 0);
        }
        Ok(None) => find_step(hwnd, 1),
        Err(problem) => set_find_status(hwnd, problem),
    }
}

/// Erstatter alle fund på én gang, som ét trin der kan fortrydes.
unsafe fn find_replace_all(hwnd: HWND) {
    let Some(inputs) = find_inputs(hwnd) else { return };
    if inputs.needle.is_empty() {
        return;
    }
    let text = doc::get_text(inputs.edit);
    let status = match find::replace_all(&text, &inputs.query(), &inputs.with) {
        Ok((_, 0)) => lang::s().none_found.to_string(),
        Ok((replaced, count)) => {
            let caret = doc::selection(inputs.edit).0.clamp(0, replaced.len() as i32);
            doc::replace_undoable(inputs.edit, &replaced, (caret, caret));
            lang::fill(lang::s().replaced, &[&count.to_string()])
        }
        Err(problem) => problem,
    };
    set_find_status(hwnd, status);
}

unsafe fn find_action(hwnd: HWND, button: FindButton) {
    match button {
        FindButton::Prev => find_step(hwnd, -1),
        FindButton::Next => find_step(hwnd, 1),
        FindButton::Case => {
            if let Some(bar) = win(hwnd).and_then(|w| w.find.as_mut()) {
                bar.match_case = !bar.match_case;
            }
            find_step(hwnd, 0);
        }
        FindButton::Regex => {
            if let Some(bar) = win(hwnd).and_then(|w| w.find.as_mut()) {
                bar.regex = !bar.regex;
            }
            find_step(hwnd, 0);
        }
        FindButton::Word => {
            // Samme kontakt som i indstillingerne.
            let s = &mut app().settings;
            s.whole_word = !s.whole_word;
            s.save();
            apply_settings(Change::Find);
        }
        FindButton::Close => find_close(hwnd),
        FindButton::Replace => find_replace(hwnd),
        FindButton::ReplaceAll => find_replace_all(hwnd),
    }
}

/// Taster i søgelinjens felter: Enter søger eller erstatter, Tab skifter felt, og Esc
/// lukker linjen (også når markøren står i teksten). Returnerer true hvis tasten er brugt.
unsafe fn find_key(hwnd: HWND, focus: HWND, key: u16, shift: bool) -> bool {
    let Some(bar) = win(hwnd).and_then(|w| w.find.as_ref()) else { return false };
    let (find, replace) = (bar.find, bar.replace);
    if key == VK_ESCAPE {
        find_close(hwnd);
        return true;
    }
    if focus != find && focus != replace {
        return false;
    }
    match key {
        VK_RETURN if focus == replace => find_replace(hwnd),
        VK_RETURN => find_step(hwnd, if shift { -1 } else { 1 }),
        VK_TAB => {
            let other = if focus == find { replace } else { find };
            SetFocus(other);
            SendMessageW(other, EM_SETSEL, 0, -1);
        }
        _ => return false,
    }
    true
}

unsafe fn draw_find(hdc: HDC, w: &Win, width: i32, height: i32) {
    let Some(bar) = w.find.as_ref() else { return };
    let theme = app().theme;
    let (dpi, top) = (w.dpi, w.strip_height());
    let f = fonts(dpi);
    // Alt tegnes i en bitmap der starter ved linjens top, så koordinaterne rykkes op.
    let moved = |r: &RECT| RECT { left: r.left, top: r.top - top, right: r.right, bottom: r.bottom - top };
    fill(hdc, &RECT { left: 0, top: 0, right: width, bottom: height }, theme.editor_bg);
    fill(hdc, &RECT { left: 0, top: height - 1, right: width, bottom: height }, theme.sep);
    SetBkMode(hdc, TRANSPARENT as _);
    SelectObject(hdc, f.ui);

    let layout = find_layout(w);
    let label = |hdc: HDC, s: &str, r: &RECT, color: u32, format: u32| {
        let text: Vec<u16> = s.encode_utf16().collect();
        if !text.is_empty() {
            let mut r = *r;
            SetTextColor(hdc, color);
            DrawTextW(hdc, text.as_ptr(), text.len() as i32, &mut r, DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX | format);
        }
    };
    let words = lang::s();
    for (name, field) in [(words.find, layout.find), (words.replace, layout.replace)] {
        let field = moved(&field);
        fill(hdc, &field, theme.hover);
        label(hdc, name, &RECT { left: scale(dpi, 12), right: field.left, ..field }, theme.dim, 0);
    }
    label(hdc, &bar.status, &moved(&layout.status), theme.dim, 0);

    for (button, r) in &layout.buttons {
        let r = moved(r);
        let hot = w.hover == Hit::Find(*button);
        let on = match button {
            FindButton::Case => bar.match_case,
            FindButton::Regex => bar.regex,
            FindButton::Word => app().settings.whole_word,
            _ => false,
        };
        let wide_button = matches!(button, FindButton::Replace | FindButton::ReplaceAll);
        if on {
            fill(hdc, &r, theme.accent);
        } else if hot {
            fill(hdc, &r, theme.close_hover);
        } else if wide_button {
            fill(hdc, &r, theme.hover);
        }
        let color = if on { theme.on_accent } else if hot || wide_button { theme.text } else { theme.dim };
        match button {
            FindButton::Prev => glyph(hdc, f.icon, GLYPH_UP, &r, color),
            FindButton::Next => glyph(hdc, f.icon, GLYPH_DOWN, &r, color),
            FindButton::Close => glyph(hdc, f.icon, GLYPH_CLOSE, &r, color),
            FindButton::Case => label(hdc, "Aa", &r, color, DT_CENTER),
            FindButton::Regex => label(hdc, ".*", &r, color, DT_CENTER),
            FindButton::Word => label(hdc, words.word, &r, color, DT_CENTER),
            FindButton::Replace => label(hdc, words.replace, &r, color, DT_CENTER),
            FindButton::ReplaceAll => label(hdc, words.replace_all, &r, color, DT_CENTER),
        }
        SelectObject(hdc, f.ui);
    }
}
