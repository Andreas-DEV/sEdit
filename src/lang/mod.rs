//! Sprog: alle tekster brugeren ser, på hvert af de sprog appen taler.
//!
//! Hvert sprog er en `Strings` i sin egen fil, og `s()` giver det aktuelle. Sproget
//! følger Windows' visningssprog, medmindre brugeren har valgt et i indstillingerne.
//! Tekster med `{0}`, `{1}` … udfyldes med `fill`, og `&` i en knaptekst står foran
//! bogstavtasten der vælger knappen, som i Windows' egne dialoger.

use std::sync::atomic::{AtomicUsize, Ordering};

use windows_sys::Win32::Globalization::{GetUserDefaultUILanguage, GetUserPreferredUILanguages, MUI_LANGUAGE_ID};

mod cs;
mod da;
mod de;
mod en;
mod es;
mod fi;
mod fr;
mod it;
mod ja;
mod ko;
mod nb;
mod nl;
mod pl;
mod pt;
mod ru;
mod sv;
mod tr;
mod uk;
mod zh_hans;
mod zh_hant;

macro_rules! strings {
    ($($field:ident),* $(,)?) => {
        /// Teksterne på ét sprog.
        pub struct Strings {
            $(pub $field: &'static str,)*
        }

        #[cfg(test)]
        impl Strings {
            fn fields(&self) -> Vec<(&'static str, &'static str)> {
                vec![$((stringify!($field), self.$field),)*]
            }
        }
    };
}

strings! {
    // Sprogets navn på engelsk; sådan står det altid i indstillingerne, så man kan finde
    // tilbage, hvis man har valgt et sprog man ikke kan læse.
    name,
    // Kommandoer.
    save, save_as, new_tab, open_file, close_tab, next_tab, prev_tab, wrap_lines, space_comments,
    settings, find_replace, find_next, find_prev,
    // Højrekliksmenuen.
    undo, cut, copy, paste, delete, select_all,
    // Dialogknapper; `&` står foran bogstavtasten.
    yes, no, cancel, save_it, dont_save, ok, reload, save_mine, keep_mine,
    // Beskeder.
    conflict, could_not_open, could_not_save, session_not_saved, save_changes, changed_outside,
    internal_error,
    all_files, untitled,
    // Søgelinjen.
    find, replace, replace_all, word, none_found, invalid_regex, of_count, replaced,
    // Indstillinger.
    general, shortcuts, theme, font, font_size, tab_width, language, wrap_long, color_code,
    line_numbers, whole_words, follow_windows, light, dark, chars, hint, press_keys, close, reset,
    press_new, all_reset, shortcut_removed, shortcut_set, shortcut_taken,
    // Hvorfor en genvej ikke kan bruges.
    need_modifier, no_ctrl_alt, no_alt_f4, reserved, no_ctrl_digit,
    // Tastnavne.
    key_space, key_left, key_up, key_right, key_down, key_other,
}

/// Sprogene i den rækkefølge de vises. Valget gemmes som plads + 1, så nye sprog
/// skal føjes til sidst.
static LANGS: [&Strings; 20] = [
    &da::S,
    &en::S,
    &de::S,
    &fr::S,
    &es::S,
    &pt::S,
    &it::S,
    &nl::S,
    &sv::S,
    &nb::S,
    &fi::S,
    &pl::S,
    &cs::S,
    &tr::S,
    &ru::S,
    &uk::S,
    &ja::S,
    &ko::S,
    &zh_hans::S,
    &zh_hant::S,
];
const ENGLISH: usize = 1;

static CURRENT: AtomicUsize = AtomicUsize::new(0);

/// Teksterne på det aktuelle sprog.
pub fn s() -> &'static Strings {
    LANGS[CURRENT.load(Ordering::Relaxed)]
}

/// Det aktuelle sprogs plads; skifter når sproget gør.
pub fn current() -> usize {
    CURRENT.load(Ordering::Relaxed)
}

/// Antal valg i indstillingerne: "Følg Windows" og hvert sprog.
pub const CHOICES: u32 = LANGS.len() as u32 + 1;

/// Navnet på et valg i indstillingerne. 0 er "Følg Windows".
pub fn choice_name(choice: u32) -> &'static str {
    match choice {
        0 => s().follow_windows,
        _ => LANGS[(choice as usize - 1).min(LANGS.len() - 1)].name,
    }
}

/// Skifter sprog. 0 følger Windows; ellers er det sprogets plads + 1.
pub fn select(choice: u32) {
    let index = match choice {
        0 => unsafe { from_windows() },
        _ => (choice as usize - 1).min(LANGS.len() - 1),
    };
    CURRENT.store(index, Ordering::Relaxed);
}

/// Det første af brugerens foretrukne visningssprog, som appen taler; ellers engelsk.
unsafe fn from_windows() -> usize {
    let (mut count, mut len) = (0u32, 0u32);
    GetUserPreferredUILanguages(MUI_LANGUAGE_ID, &mut count, std::ptr::null_mut(), &mut len);
    let mut buffer = vec![0u16; len as usize];
    if len > 0 && GetUserPreferredUILanguages(MUI_LANGUAGE_ID, &mut count, buffer.as_mut_ptr(), &mut len) != 0 {
        // Listen er sprog-id'er som hex ("0406"), adskilt af NUL.
        let preferred = buffer
            .split(|&unit| unit == 0)
            .filter_map(|id| u16::from_str_radix(&String::from_utf16_lossy(id), 16).ok())
            .find_map(from_langid);
        if let Some(index) = preferred {
            return index;
        }
    }
    from_langid(GetUserDefaultUILanguage()).unwrap_or(ENGLISH)
}

/// Sprogets plads i `LANGS` ud fra et Windows-sprog-id, eller None hvis appen ikke taler det.
fn from_langid(id: u16) -> Option<usize> {
    Some(match id & 0x3FF {
        0x06 => 0,
        0x09 => 1,
        0x07 => 2,
        0x0C => 3,
        0x0A => 4,
        0x16 => 5,
        0x10 => 6,
        0x13 => 7,
        0x1D => 8,
        0x14 => 9,
        0x0B => 10,
        0x15 => 11,
        0x05 => 12,
        0x1F => 13,
        0x19 => 14,
        0x22 => 15,
        0x11 => 16,
        0x12 => 17,
        // Kinesisk: Taiwan, Hongkong og Macao skriver traditionelt, resten forenklet.
        0x04 if matches!(id, 0x0404 | 0x0C04 | 0x1404 | 0x7C04) => 19,
        0x04 => 18,
        _ => return None,
    })
}

/// Sætter `args` ind på pladserne `{0}`, `{1}` … i en tekst.
pub fn fill(template: &str, args: &[&str]) -> String {
    let mut out = String::with_capacity(template.len() + 32);
    let mut rest = template;
    while let Some(at) = rest.find('{') {
        out.push_str(&rest[..at]);
        let tail = &rest[at..];
        match tail.as_bytes() {
            [_, digit, b'}', ..] if digit.is_ascii_digit() => {
                out.push_str(args.get((digit - b'0') as usize).copied().unwrap_or(""));
                rest = &tail[3..];
            }
            _ => {
                out.push('{');
                rest = &tail[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// En knaptekst som den vises: uden `&`.
pub fn plain(label: &str) -> String {
    label.replace('&', "")
}

/// Bogstavtasten der vælger en knap: tegnet efter `&`, eller 0 hvis teksten ikke har en.
pub fn mnemonic(label: &str) -> u8 {
    label
        .split_once('&')
        .and_then(|(_, after)| after.bytes().next())
        .filter(u8::is_ascii_alphanumeric)
        .map_or(0, |key| key.to_ascii_uppercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn placeholders(text: &str) -> Vec<u8> {
        let mut found: Vec<u8> = text.as_bytes().windows(3).filter(|w| w[0] == b'{' && w[1].is_ascii_digit() && w[2] == b'}').map(|w| w[1]).collect();
        found.sort();
        found
    }

    #[test]
    fn every_language_fills_the_same_placeholders() {
        for lang in LANGS {
            for ((field, danish), (_, text)) in da::S.fields().into_iter().zip(lang.fields()) {
                assert!(!text.is_empty(), "{}: {field} er tom", lang.name);
                assert_eq!(placeholders(danish), placeholders(text), "{}: {field}", lang.name);
            }
        }
    }

    #[test]
    fn button_keys_do_not_collide() {
        for lang in LANGS {
            for group in [[lang.yes, lang.no, lang.cancel], [lang.save_it, lang.dont_save, lang.cancel], [lang.reload, lang.save_mine, lang.keep_mine]] {
                let keys: Vec<u8> = group.iter().map(|label| mnemonic(label)).filter(|&key| key != 0).collect();
                let mut unique = keys.clone();
                unique.sort();
                unique.dedup();
                assert_eq!(keys.len(), unique.len(), "{}: {group:?}", lang.name);
            }
        }
    }

    #[test]
    fn button_labels_lose_their_marker() {
        assert_eq!((plain("Gem &mit"), mnemonic("Gem &mit")), ("Gem mit".to_string(), b'M'));
        assert_eq!((plain("是(&Y)"), mnemonic("是(&Y)")), ("是(Y)".to_string(), b'Y'));
        assert_eq!((plain("Да"), mnemonic("Да")), ("Да".to_string(), 0));
    }

    #[test]
    fn placeholders_are_filled_once() {
        assert_eq!(fill("{1} af {0}", &["a", "b"]), "b af a");
        // Et filnavn der ligner en plads, sættes ikke ind igen, og en løs { står urørt.
        assert_eq!(fill("{0} og {1} {", &["{1}", "x"]), "{1} og x {");
        assert_eq!(fill("{2}", &["a"]), "");
    }

    #[test]
    fn windows_languages_map_to_ours() {
        assert_eq!(from_langid(0x0406), Some(0));
        assert_eq!(LANGS[from_langid(0x0809).unwrap()].name, "English");
        assert_eq!(LANGS[from_langid(0x0C0C).unwrap()].name, "French");
        assert_eq!(LANGS[from_langid(0x0416).unwrap()].name, "Portuguese");
        assert_eq!(LANGS[from_langid(0x0814).unwrap()].name, "Norwegian");
        assert_eq!(LANGS[from_langid(0x0804).unwrap()].name, "Chinese (Simplified)");
        assert_eq!(LANGS[from_langid(0x0404).unwrap()].name, "Chinese (Traditional)");
        assert_eq!(LANGS[from_langid(0x0C04).unwrap()].name, "Chinese (Traditional)");
        // Arabisk tales ikke (endnu).
        assert_eq!(from_langid(0x0401), None);
    }
}
