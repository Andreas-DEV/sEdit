//! Farver koden i et tekstfelt.
//!
//! Teksten deles op af `syntax`, og farverne lægges på gennem RichEdits tekstmodel
//! (ITextDocument), så hverken markering, rulleposition eller fortryd-historik røres.
//!
//! For at store filer ikke skal koste noget, farves kun det der kan ses plus lidt til,
//! lige før feltet tegnes. Resten farves i små bidder, når der ikke sker andet.
//! `valid` er det stykke af teksten, hvor farverne passer med `tokens`; efter en rettelse
//! farves kun det stykke om, hvor opdelingen faktisk har ændret sig.
//!
//! Tilstanden hænger på tekstfeltet selv (en vinduesegenskab), så den følger med fanen
//! mellem vinduer, og filer uden et kendt sprog får aldrig nogen.

use std::ffi::c_void;
use std::mem::transmute_copy;
use std::ptr::{null, null_mut};

use windows_sys::core::GUID;
use windows_sys::w;
use windows_sys::Win32::Foundation::{HWND, LPARAM, POINT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::InvalidateRect;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    GetClientRect, GetPropW, GetQueueStatus, IsWindowVisible, KillTimer, RemovePropW, SendMessageW,
    SetPropW, QS_INPUT,
    SetTimer, WM_CHAR, WM_CLEAR, WM_CUT, WM_KEYDOWN, WM_SETTEXT, WM_SYSKEYDOWN, WM_UNDO,
};

use crate::doc;
use crate::syntax::{self, Lang, Token};
use crate::theme::Theme;

/// Tekstfeltets egen timer til at farve resten af filen. RichEdit bruger selv små numre.
pub const TIMER: usize = 0x5ED1;
const TIMER_MS: u32 = 15;
/// Så mange tegn farves ud over det synlige, så almindelig rulning ikke når kanten.
const MARGIN: usize = 4_000;
/// Så mange tegn farves ad gangen i baggrunden.
const CHUNK: usize = 2_000;
/// Større filer end dette farves ikke.
const LIMIT: usize = 1_000_000;

const PROPERTY: *const u16 = w!("sEdit.Highlight");

const WM_USER: u32 = 0x0400;
const WM_UNICHAR: u32 = 0x0109;
const WM_IME_COMPOSITION: u32 = 0x010F;
const WM_IME_CHAR: u32 = 0x0286;
const EM_LINEINDEX: u32 = 0x00BB;
const EM_REPLACESEL: u32 = 0x00C2;
const EM_UNDO: u32 = 0x00C7;
const EM_GETFIRSTVISIBLELINE: u32 = 0x00CE;
const EM_CHARFROMPOS: u32 = 0x00D7;
const EM_GETOLEINTERFACE: u32 = WM_USER + 60;
pub const EM_PASTESPECIAL: u32 = WM_USER + 64;
const EM_REDO: u32 = WM_USER + 84;
const EM_SETTEXTEX: u32 = WM_USER + 97;

// Text Object Model (tom.h). windows-sys har ingen COM, så metoderne kaldes gennem
// deres plads i vtabellen: IUnknown fylder 0-2 og IDispatch 3-6.
type Raw = *mut c_void;
const IID_ITEXTDOCUMENT: GUID = GUID {
    data1: 0x8CC4_97C0,
    data2: 0xA1DF,
    data3: 0x11CE,
    data4: [0x80, 0x98, 0x00, 0xAA, 0x00, 0x47, 0xBE, 0x5D],
};
const UNKNOWN_QUERY_INTERFACE: usize = 0;
const UNKNOWN_RELEASE: usize = 2;
const DOCUMENT_UNDO: usize = 22;
const DOCUMENT_RANGE: usize = 24;
const RANGE_GET_FONT: usize = 18;
const RANGE_SET_RANGE: usize = 28;
const FONT_SET_FORE_COLOR: usize = 25;
const TOM_SUSPEND: i32 = -9_999_995;
const TOM_RESUME: i32 = -9_999_994;

unsafe fn method<F: Copy>(object: Raw, slot: usize) -> F {
    let vtable = *(object as *const *const Raw);
    transmute_copy(&*vtable.add(slot))
}

unsafe fn release(object: Raw) {
    if !object.is_null() {
        let f: unsafe extern "system" fn(Raw) -> u32 = method(object, UNKNOWN_RELEASE);
        f(object);
    }
}

/// Adgangen til feltets tekstmodel: dokumentet, et område i det og områdets skrift.
/// Skriften følger området, så ét sæt objekter kan genbruges til alle stykker.
struct Model {
    document: Raw,
    range: Raw,
    font: Raw,
}

impl Model {
    unsafe fn open(edit: HWND) -> Option<Model> {
        let mut ole: Raw = null_mut();
        SendMessageW(edit, EM_GETOLEINTERFACE, 0, &mut ole as *mut Raw as LPARAM);
        if ole.is_null() {
            return None;
        }
        let mut model = Model { document: null_mut(), range: null_mut(), font: null_mut() };
        let query: unsafe extern "system" fn(Raw, *const GUID, *mut Raw) -> i32 = method(ole, UNKNOWN_QUERY_INTERFACE);
        query(ole, &IID_ITEXTDOCUMENT, &mut model.document);
        release(ole);
        if model.document.is_null() {
            return None;
        }
        let range: unsafe extern "system" fn(Raw, i32, i32, *mut Raw) -> i32 = method(model.document, DOCUMENT_RANGE);
        range(model.document, 0, 0, &mut model.range);
        if !model.range.is_null() {
            let font: unsafe extern "system" fn(Raw, *mut Raw) -> i32 = method(model.range, RANGE_GET_FONT);
            font(model.range, &mut model.font);
        }
        // Mangler noget, rydder Drop op efter det der nåede at blive hentet.
        (!model.font.is_null()).then_some(model)
    }

    unsafe fn undo(&self, mode: i32) {
        let f: unsafe extern "system" fn(Raw, i32, *mut i32) -> i32 = method(self.document, DOCUMENT_UNDO);
        f(self.document, mode, null_mut());
    }

    unsafe fn color(&self, from: usize, to: usize, color: u32) {
        let set_range: unsafe extern "system" fn(Raw, i32, i32) -> i32 = method(self.range, RANGE_SET_RANGE);
        let set_color: unsafe extern "system" fn(Raw, i32) -> i32 = method(self.font, FONT_SET_FORE_COLOR);
        set_range(self.range, from as i32, to as i32);
        set_color(self.font, color as i32);
    }
}

impl Drop for Model {
    fn drop(&mut self) {
        unsafe {
            release(self.font);
            release(self.range);
            release(self.document);
        }
    }
}

struct State {
    lang: Option<Lang>,
    /// Teksten som `tokens` er lavet ud fra.
    text: Vec<u16>,
    tokens: Vec<Token>,
    /// Stykket hvor farverne i feltet passer med `tokens`.
    valid: (usize, usize),
    /// Teksten er ændret, mens feltet var skjult; alt laves om, når det skal tegnes.
    stale: bool,
    /// Vi er selv i gang med at farve, så beskeder undervejs skal ikke starte en ny runde.
    busy: bool,
    model: Option<Model>,
}

unsafe fn state<'a>(edit: HWND) -> Option<&'a mut State> {
    (GetPropW(edit, PROPERTY) as *mut State).as_mut()
}

/// Vælger sproget for feltet. `None` fjerner farverne igen.
pub unsafe fn set_lang(edit: HWND, lang: Option<Lang>) {
    if state(edit).is_none() {
        if lang.is_none() {
            return;
        }
        let fresh = Box::new(State {
            lang: None,
            text: Vec::new(),
            tokens: Vec::new(),
            valid: (0, 0),
            stale: true,
            busy: false,
            model: None,
        });
        SetPropW(edit, PROPERTY, Box::into_raw(fresh) as _);
    }
    let Some(st) = state(edit) else { return };
    if st.lang == lang {
        return;
    }
    st.lang = lang;
    st.stale = true;
    if lang.is_none() {
        resync(st, edit);
        st.text = Vec::new();
        KillTimer(edit, TIMER);
    }
    InvalidateRect(edit, null(), 0);
}

/// Farverne er væk (tema eller skrift er skiftet) og skal lægges på igen.
pub unsafe fn invalidate(edit: HWND) {
    if let Some(st) = state(edit) {
        st.stale = true;
        InvalidateRect(edit, null(), 0);
    }
}

/// Kaldes når tekstfeltet nedlægges.
pub unsafe fn destroy(edit: HWND) {
    let pointer = RemovePropW(edit, PROPERTY) as *mut State;
    if !pointer.is_null() {
        KillTimer(edit, TIMER);
        drop(Box::from_raw(pointer));
    }
}

/// Beskeder der kan ændre teksten. Efter dem ser `text_changed` efter, om den er ændret.
pub fn may_change_text(msg: u32) -> bool {
    matches!(
        msg,
        WM_CHAR
            | WM_UNICHAR
            | WM_KEYDOWN
            | WM_SYSKEYDOWN
            | WM_IME_COMPOSITION
            | WM_IME_CHAR
            | WM_CUT
            | WM_CLEAR
            | WM_UNDO
            | WM_SETTEXT
            | EM_UNDO
            | EM_REDO
            | EM_REPLACESEL
            | EM_PASTESPECIAL
            | EM_SETTEXTEX
    )
}

/// Teksten kan være ændret: find ud af hvor, og farv det stykke om.
pub unsafe fn text_changed(edit: HWND) {
    let Some(st) = state(edit) else { return };
    if st.lang.is_none() || st.busy || st.stale {
        return;
    }
    if IsWindowVisible(edit) == 0 {
        st.stale = true;
        return;
    }
    let text = doc::get_text(edit);
    if text == st.text {
        return;
    }
    let tokens = tokenize(st.lang, &text);
    let change = Change::between(&st.text, &text, &st.tokens, &tokens);
    let kept = change.remap(st.valid);
    st.text = text;
    st.tokens = tokens;

    let window = window(edit, st.text.len());
    let (from, to) = (change.from.max(window.0), change.to.min(window.1));
    if from < to {
        paint(st, edit, from, to);
    }
    st.valid = if change.from >= window.0 && change.to <= window.1 {
        // Det hele er farvet om. Hænger det sammen med det der var i orden, vokser det.
        if change.from <= kept.1 && change.to >= kept.0 {
            (kept.0.min(change.from), kept.1.max(change.to))
        } else {
            kept
        }
    } else if change.from >= window.0 {
        // Ændringen rækker længere ned end vi farver nu (fx en ny `/*`). Alt efter
        // det farvede venter, til det skal vises.
        if kept.0 <= change.from && kept.1 >= change.from {
            (kept.0, to.max(change.from))
        } else if from < to {
            (from, to)
        } else {
            (kept.0, kept.1.min(change.from).max(kept.0))
        }
    } else if from < to {
        (from, to)
    } else {
        (0, 0)
    };
    schedule(st, edit);
}

/// Kaldes lige før feltet tegnes: sørger for at det synlige er farvet.
pub unsafe fn before_paint(edit: HWND) {
    let Some(st) = state(edit) else { return };
    if st.lang.is_none() || st.busy {
        return;
    }
    // Sikkerhedsnet: er teksten ændret ad en vej vi ikke holder øje med, laves alt om.
    if st.stale || doc::text_len(edit) != st.text.len() {
        resync(st, edit);
    }
    let len = st.text.len();
    let seen = visible(edit, len);
    if seen.0 >= st.valid.0 && seen.1 <= st.valid.1 {
        return;
    }
    let window = (seen.0.saturating_sub(MARGIN), (seen.1 + MARGIN).min(len));
    if window.0 <= st.valid.1 && window.1 >= st.valid.0 && st.valid.0 < st.valid.1 {
        // Overlapper det der er farvet: kun enderne mangler.
        if window.0 < st.valid.0 {
            paint(st, edit, window.0, st.valid.0);
        }
        if window.1 > st.valid.1 {
            paint(st, edit, st.valid.1, window.1);
        }
        st.valid = (st.valid.0.min(window.0), st.valid.1.max(window.1));
    } else {
        paint(st, edit, window.0, window.1);
        st.valid = window;
    }
    schedule(st, edit);
}

/// Farver endnu en bid af resten af filen. Kaldes af feltets timer.
pub unsafe fn idle(edit: HWND) {
    let Some(st) = state(edit) else {
        KillTimer(edit, TIMER);
        return;
    };
    let len = st.text.len();
    let (from, to) = st.valid;
    if st.lang.is_none() || st.stale || st.busy || from >= to || IsWindowVisible(edit) == 0 {
        // Intet at bygge videre på; næste tegning starter forfra.
        KillTimer(edit, TIMER);
    } else if GetQueueStatus(QS_INPUT) >> 16 != 0 {
        // Brugeren skriver eller ruller: baggrundsarbejdet venter til næste gang.
    } else if to < len {
        let next = (to + CHUNK).min(len);
        paint(st, edit, to, next);
        st.valid.1 = next;
    } else if from > 0 {
        let next = from.saturating_sub(CHUNK);
        paint(st, edit, next, from);
        st.valid.0 = next;
    } else {
        KillTimer(edit, TIMER);
    }
}

unsafe fn schedule(st: &State, edit: HWND) {
    if st.valid.0 < st.valid.1 && st.valid != (0, st.text.len()) {
        SetTimer(edit, TIMER, TIMER_MS, None);
    }
}

fn tokenize(lang: Option<Lang>, text: &[u16]) -> Vec<Token> {
    match lang {
        Some(lang) if text.len() <= LIMIT => syntax::tokenize(lang, text),
        _ => Vec::new(),
    }
}

/// Læser teksten forfra og nulstiller alle farver. Bagefter er intet farvet.
unsafe fn resync(st: &mut State, edit: HWND) {
    st.text = doc::get_text(edit);
    st.tokens = tokenize(st.lang, &st.text);
    st.stale = false;
    st.valid = (0, 0);
    let theme = crate::ui::theme();
    edit_colors(st, edit, |model| model.color(0, i32::MAX as usize, theme.editor_fg));
}

/// Lægger farverne på stykket `from..to`.
unsafe fn paint(st: &mut State, edit: HWND, from: usize, to: usize) {
    let theme = crate::ui::theme();
    let first = st.tokens.partition_point(|t| t.end <= from);
    let tokens = &st.tokens[first..];
    edit_colors_with(&mut st.model, &mut st.busy, edit, |model| {
        model.color(from, to, theme.editor_fg);
        for token in tokens.iter().take_while(|t| t.start < to) {
            model.color(token.start.max(from), token.end.min(to), theme.syntax(token.kind));
        }
    });
}

unsafe fn edit_colors(st: &mut State, edit: HWND, change: impl FnOnce(&Model)) {
    edit_colors_with(&mut st.model, &mut st.busy, edit, change);
}

/// Kører en farveændring, så den hverken tæller som en rettelse af teksten eller
/// havner i fortryd-historikken.
unsafe fn edit_colors_with(model: &mut Option<Model>, busy: &mut bool, edit: HWND, change: impl FnOnce(&Model)) {
    if model.is_none() {
        *model = Model::open(edit);
    }
    let Some(model) = model.as_ref() else { return };
    let modified = doc::is_modified(edit);
    *busy = true;
    model.undo(TOM_SUSPEND);
    change(model);
    model.undo(TOM_RESUME);
    *busy = false;
    doc::set_modified(edit, modified);
}

/// Tegnpositionerne der kan ses i feltet lige nu.
unsafe fn visible(edit: HWND, len: usize) -> (usize, usize) {
    let line = SendMessageW(edit, EM_GETFIRSTVISIBLELINE, 0, 0);
    let first = SendMessageW(edit, EM_LINEINDEX, line as WPARAM, 0).max(0) as usize;
    let mut client = RECT { left: 0, top: 0, right: 0, bottom: 0 };
    GetClientRect(edit, &mut client);
    let corner = POINT { x: (client.right - 1).max(0), y: (client.bottom - 1).max(0) };
    let last = SendMessageW(edit, EM_CHARFROMPOS, 0, &corner as *const POINT as LPARAM).max(0) as usize;
    let first = first.min(len);
    (first, (last + 1).clamp(first, len))
}

/// Det synlige plus margenen.
unsafe fn window(edit: HWND, len: usize) -> (usize, usize) {
    let seen = visible(edit, len);
    (seen.0.saturating_sub(MARGIN), (seen.1 + MARGIN).min(len))
}

/// Forskellen mellem to udgaver af teksten og dens opdeling.
#[derive(Debug, PartialEq)]
struct Change {
    /// Så mange tegn er ens i starten og i slutningen af de to tekster.
    prefix: usize,
    suffix: usize,
    old_len: usize,
    new_len: usize,
    /// Stykket af den nye tekst, hvor farverne kan være forkerte.
    from: usize,
    to: usize,
}

impl Change {
    fn between(old: &[u16], new: &[u16], old_tokens: &[Token], new_tokens: &[Token]) -> Change {
        let shortest = old.len().min(new.len());
        let prefix = old.iter().zip(new).take_while(|(a, b)| a == b).count();
        let suffix = old.iter().rev().zip(new.iter().rev()).take(shortest - prefix).take_while(|(a, b)| a == b).count();
        let delta = new.len() as isize - old.len() as isize;

        // Stykker der er ens i starten, og stykker der er ens i slutningen bortset fra
        // at de er rykket med ændringens længde.
        let fewest = old_tokens.len().min(new_tokens.len());
        let same_first = old_tokens.iter().zip(new_tokens).take_while(|(a, b)| a == b).count();
        let moved = |t: &Token| Token {
            start: (t.start as isize + delta) as usize,
            end: (t.end as isize + delta) as usize,
            kind: t.kind,
        };
        let same_last = old_tokens
            .iter()
            .rev()
            .zip(new_tokens.iter().rev())
            .take(fewest - same_first)
            .take_while(|(a, b)| a.start >= old.len() - suffix && moved(a) == **b)
            .count();

        // Nyskrevet tekst arver farven fra nabotegnet, så den er altid med.
        let mut from = prefix;
        let mut to = new.len() - suffix;
        let old_changed = &old_tokens[same_first..old_tokens.len() - same_last];
        let new_changed = &new_tokens[same_first..new_tokens.len() - same_last];
        if let (Some(first), Some(last)) = (new_changed.first(), new_changed.last()) {
            from = from.min(first.start);
            to = to.max(last.end);
        }
        if let (Some(first), Some(last)) = (old_changed.first(), old_changed.last()) {
            from = from.min(first.start);
            to = to.max((last.end as isize + delta).max(0) as usize);
        }
        let to = to.min(new.len());
        Change { prefix, suffix, old_len: old.len(), new_len: new.len(), from: from.min(to), to }
    }

    /// Flytter et stykke af den gamle tekst over i den nye. Det der lå i selve
    /// ændringen, skæres fra.
    fn remap(&self, range: (usize, usize)) -> (usize, usize) {
        let old_tail = self.old_len - self.suffix;
        let new_tail = self.new_len - self.suffix;
        let start = if range.0 <= self.prefix {
            range.0
        } else if range.0 >= old_tail {
            range.0 - old_tail + new_tail
        } else {
            new_tail
        };
        let end = if range.1 <= self.prefix {
            range.1
        } else if range.1 >= old_tail {
            range.1 - old_tail + new_tail
        } else {
            self.prefix
        };
        (start, end.max(start))
    }
}

impl Theme {
    pub fn syntax(&self, kind: syntax::Kind) -> u32 {
        match kind {
            syntax::Kind::Keyword => self.keyword,
            syntax::Kind::Str => self.string,
            syntax::Kind::Number => self.number,
            syntax::Kind::Comment => self.comment,
            syntax::Kind::Type => self.type_name,
            syntax::Kind::Function => self.function,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn w(s: &str) -> Vec<u16> {
        s.encode_utf16().collect()
    }

    fn change(old: &str, new: &str) -> Change {
        let (old, new) = (w(old), w(new));
        let old_tokens = syntax::tokenize(Lang::JavaScript, &old);
        let new_tokens = syntax::tokenize(Lang::JavaScript, &new);
        Change::between(&old, &new, &old_tokens, &new_tokens)
    }

    #[test]
    fn typing_in_a_name_recolors_only_what_was_typed() {
        let c = change("let abc = 1; // slut", "let abXc = 1; // slut");
        assert_eq!((c.from, c.to), (6, 7));
    }

    #[test]
    fn typing_in_a_word_recolors_the_whole_word() {
        // "le" bliver til nøgleordet "let".
        let c = change("le x = 1", "let x = 1");
        assert_eq!((c.from, c.to), (0, 3));
        // Og nøgleordet holder op med at være et.
        let c = change("let x = 1", "lett x = 1");
        assert_eq!((c.from, c.to), (0, 4));
    }

    #[test]
    fn opening_a_comment_recolors_everything_after_it() {
        let c = change("a = 1;\rb = 'x';", "a = 1;/*\rb = 'x';");
        assert_eq!((c.from, c.to), (6, 17));
    }

    #[test]
    fn deleting_text_between_tokens_recolors_nothing() {
        let c = change("let  x", "let x");
        assert_eq!(c.from, c.to);
    }

    #[test]
    fn repeated_characters_do_not_confuse_the_comparison() {
        // Præfiks og suffiks må ikke overlappe, når der indsættes et tegn magen til naboen.
        let c = change("aaa", "aaaa");
        assert_eq!((c.prefix, c.suffix, c.from, c.to), (3, 0, 3, 4));
        let c = change("1 1 1", "1 1 1 1");
        assert!(c.from <= 5 && c.to == 7, "{c:?}");
    }

    #[test]
    fn valid_range_follows_the_text_around_a_change() {
        // 10 tegn, tegn 4..6 erstattes af 5 tegn: præfiks 4, suffiks 4.
        let c = Change { prefix: 4, suffix: 4, old_len: 10, new_len: 13, from: 4, to: 9 };
        assert_eq!(c.remap((0, 10)), (0, 13));
        assert_eq!(c.remap((0, 3)), (0, 3));
        assert_eq!(c.remap((7, 10)), (10, 13));
        // Ender der lå inde i ændringen, trækkes ud til dens kant.
        assert_eq!(c.remap((0, 5)), (0, 4));
        assert_eq!(c.remap((5, 10)), (9, 13));
        assert_eq!(c.remap((5, 5)), (9, 9));
    }
}

// ---------- tabulatorbredde og linjenumre ----------

const DOCUMENT_SET_DEFAULT_TAB_STOP: usize = 14;
const RANGE_GET_INDEX: usize = 26;
const TOM_PARAGRAPH: i32 = 4;
const EM_POSFROMCHAR: u32 = WM_USER + 38;

/// Feltets tekstmodel (ITextDocument). Kalderen frigiver den med `release`.
unsafe fn open_document(edit: HWND) -> Raw {
    let mut ole: Raw = null_mut();
    SendMessageW(edit, EM_GETOLEINTERFACE, 0, &mut ole as *mut Raw as LPARAM);
    if ole.is_null() {
        return null_mut();
    }
    let mut document: Raw = null_mut();
    let query: unsafe extern "system" fn(Raw, *const GUID, *mut Raw) -> i32 = method(ole, UNKNOWN_QUERY_INTERFACE);
    query(ole, &IID_ITEXTDOCUMENT, &mut document);
    release(ole);
    document
}

/// Sætter afstanden mellem tabulatorstop, målt i punkter.
pub unsafe fn set_tab_width(edit: HWND, points: f32) {
    let document = open_document(edit);
    if document.is_null() {
        return;
    }
    let modified = doc::is_modified(edit);
    let set: unsafe extern "system" fn(Raw, f32) -> i32 = method(document, DOCUMENT_SET_DEFAULT_TAB_STOP);
    set(document, points);
    release(document);
    doc::set_modified(edit, modified);
    InvalidateRect(edit, null(), 0);
}

/// De linjer der kan ses i feltet: y-koordinaten for linjens top og linjens nummer.
/// En lang linje der er ombrudt, tæller som én og står kun med sit nummer én gang.
pub unsafe fn visible_lines(edit: HWND) -> Vec<(i32, u32)> {
    let mut lines = Vec::new();
    let document = open_document(edit);
    if document.is_null() {
        return lines;
    }
    let mut range: Raw = null_mut();
    let make: unsafe extern "system" fn(Raw, i32, i32, *mut Raw) -> i32 = method(document, DOCUMENT_RANGE);
    make(document, 0, 0, &mut range);
    if !range.is_null() {
        let set_range: unsafe extern "system" fn(Raw, i32, i32) -> i32 = method(range, RANGE_SET_RANGE);
        let get_index: unsafe extern "system" fn(Raw, i32, *mut i32) -> i32 = method(range, RANGE_GET_INDEX);
        let mut client = RECT { left: 0, top: 0, right: 0, bottom: 0 };
        GetClientRect(edit, &mut client);
        let mut line = SendMessageW(edit, EM_GETFIRSTVISIBLELINE, 0, 0).max(0);
        let mut previous = 0;
        loop {
            let start = SendMessageW(edit, EM_LINEINDEX, line as WPARAM, 0);
            if start < 0 {
                break;
            }
            let mut at = POINT { x: 0, y: 0 };
            SendMessageW(edit, EM_POSFROMCHAR, &mut at as *mut POINT as WPARAM, start);
            if at.y >= client.bottom {
                break;
            }
            let mut number = 0;
            set_range(range, start as i32, start as i32);
            get_index(range, TOM_PARAGRAPH, &mut number);
            if number > 0 && number != previous {
                lines.push((at.y, number as u32));
                previous = number;
            }
            line += 1;
        }
    }
    release(range);
    release(document);
    lines
}
