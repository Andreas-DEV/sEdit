//! Søgning i tekst. Ren tekstbehandling uden Win32, så den kan testes.
//!
//! Positionerne er UTF-16-enheder, de samme som tekstfeltet regner i, og linjeskift er
//! ét tegn (CR). Regulære udtryk køres af `regex-lite`, som arbejder på UTF-8, så
//! teksten oversættes dertil og fundenes positioner tilbage igen.

use regex_lite::{Regex, RegexBuilder};

/// Det der søges efter, og hvordan.
#[derive(Clone, Copy)]
pub struct Query<'a> {
    pub needle: &'a [u16],
    /// Om store og små bogstaver skal passe.
    pub match_case: bool,
    /// Om `needle` er et regulært udtryk.
    pub regex: bool,
    /// Om et fund skal stå som et helt ord: "port" findes ikke i "export".
    pub whole_word: bool,
}

/// Bogstaver, tal og understreg hører til et ord; alt andet skiller ord ad.
fn is_word(c: u16) -> bool {
    c == b'_' as u16 || char::from_u32(c as u32).is_some_and(char::is_alphanumeric) || (0xD800..0xE000).contains(&c)
}

/// Om fundet står for sig selv, uden ordtegn lige før eller lige efter.
fn stands_alone(text: &[u16], (start, end): Match) -> bool {
    let before = start.checked_sub(1).and_then(|i| text.get(i));
    !before.is_some_and(|&c| is_word(c)) && !text.get(end).is_some_and(|&c| is_word(c))
}

/// Et fund: start og slut, slut ikke med.
pub type Match = (usize, usize);

const CR: u16 = 13;

/// Gør et tegn til småt, så søgningen kan se bort fra store og små bogstaver.
/// Tegn der fylder mere end én enhed som små bogstaver, lades være.
fn fold(c: u16) -> u16 {
    if c < 0x80 {
        return (c as u8).to_ascii_lowercase() as u16;
    }
    let Some(character) = char::from_u32(c as u32) else { return c };
    let mut lower = character.to_lowercase();
    match (lower.next(), lower.next()) {
        (Some(single), None) if (single as u32) < 0x1_0000 => single as u32 as u16,
        _ => c,
    }
}

/// Teksten som UTF-8 med almindelige linjeskift, så `^`, `$` og `.` opfører sig som
/// man forventer. Et halvt surrogatpar bliver til ét erstatningstegn, så antallet af
/// enheder stadig passer.
fn to_utf8(text: &[u16]) -> String {
    char::decode_utf16(text.iter().map(|&c| if c == CR { 10 } else { c }))
        .map(|c| c.unwrap_or(char::REPLACEMENT_CHARACTER))
        .collect()
}

/// Oversætter stigende bytepositioner i `utf8` til UTF-16-enheder.
struct Units<'a> {
    chars: std::str::CharIndices<'a>,
    len: usize,
    byte: usize,
    unit: usize,
}

impl<'a> Units<'a> {
    fn new(utf8: &'a str) -> Units<'a> {
        Units { chars: utf8.char_indices(), len: utf8.len(), byte: 0, unit: 0 }
    }

    fn at(&mut self, byte: usize) -> usize {
        while self.byte < byte.min(self.len) {
            match self.chars.next() {
                Some((index, c)) => {
                    self.byte = index + c.len_utf8();
                    self.unit += c.len_utf16();
                }
                None => break,
            }
        }
        self.unit
    }
}

fn compile(query: &Query) -> Result<Regex, String> {
    RegexBuilder::new(&to_utf8(query.needle))
        .case_insensitive(!query.match_case)
        // ^ og $ gælder hver linje, ikke kun hele teksten.
        .multi_line(true)
        .build()
        .map_err(|_| crate::lang::s().invalid_regex.to_string())
}

/// Hvor søgeteksten står i `text`. Fundene overlapper ikke: i "aaaa" står "aa" to gange.
/// Fejler kun, hvis et regulært udtryk ikke kan forstås.
pub fn matches(text: &[u16], query: &Query) -> Result<Vec<Match>, String> {
    let mut found = all_matches(text, query)?;
    if query.whole_word {
        found.retain(|&m| stands_alone(text, m));
    }
    Ok(found)
}

fn all_matches(text: &[u16], query: &Query) -> Result<Vec<Match>, String> {
    let needle = query.needle;
    let mut found = Vec::new();
    if needle.is_empty() {
        return Ok(found);
    }
    if query.regex {
        let regex = compile(query)?;
        let utf8 = to_utf8(text);
        let mut units = Units::new(&utf8);
        for m in regex.find_iter(&utf8) {
            // Et tomt fund (som `^` eller `x*` uden x) kan hverken markeres eller erstattes.
            if m.end() > m.start() {
                let start = units.at(m.start());
                found.push((start, units.at(m.end())));
            }
        }
        return Ok(found);
    }
    if needle.len() > text.len() {
        return Ok(found);
    }
    let same = |a: u16, b: u16| if query.match_case { a == b } else { fold(a) == fold(b) };
    let last = text.len() - needle.len();
    let mut at = 0;
    while at <= last {
        if same(text[at], needle[0]) && text[at..at + needle.len()].iter().zip(needle).all(|(&a, &b)| same(a, b)) {
            found.push((at, at + needle.len()));
            at += needle.len();
        } else {
            at += 1;
        }
    }
    Ok(found)
}

/// Hvilket fund der skal vælges, når markeringen står på `start..end`.
///
/// `direction` 1 er næste, -1 er forrige, og 0 er "det fund markøren står på eller det
/// første efter", som bruges mens man skriver i søgefeltet. Søgningen går rundt ved
/// tekstens ender.
pub fn pick(found: &[Match], start: usize, end: usize, direction: i32) -> Option<usize> {
    if found.is_empty() {
        return None;
    }
    let on_match = found.contains(&(start, end));
    let index = match direction {
        1 => found.iter().position(|&(at, _)| if on_match { at > start } else { at >= start }).unwrap_or(0),
        -1 => found.iter().rposition(|&(at, _)| at < start).unwrap_or(found.len() - 1),
        _ => found.iter().position(|&(at, _)| at >= start).unwrap_or(0),
    };
    Some(index)
}

/// Erstatningsteksten for et regulært udtryk: `\n` og `\t` bliver til linjeskift og
/// tabulator, og `$1`, `${navn}` og `$0` sættes ind af `regex-lite`.
fn template(with: &[u16]) -> String {
    let raw = to_utf8(with);
    let mut out = String::with_capacity(raw.len());
    let mut chars = raw.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('\\') => out.push('\\'),
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    out
}

/// Tilbage til tekstfeltets form: UTF-16 med CR som linjeskift.
fn to_utf16(text: &str) -> Vec<u16> {
    text.encode_utf16().map(|c| if c == 10 { CR } else { c }).collect()
}

/// Det fundet på `range` skal erstattes med, eller None hvis `range` ikke er et fund.
/// For et regulært udtryk sættes grupperne ($1, $2 ...) ind.
pub fn replacement(text: &[u16], query: &Query, range: Match, with: &[u16]) -> Result<Option<Vec<u16>>, String> {
    if !matches(text, query)?.contains(&range) {
        return Ok(None);
    }
    if !query.regex {
        return Ok(Some(with.to_vec()));
    }
    let regex = compile(query)?;
    let utf8 = to_utf8(text);
    let mut units = Units::new(&utf8);
    for captures in regex.captures_iter(&utf8) {
        let whole = captures.get(0).expect("gruppe 0 findes altid");
        if units.at(whole.start()) == range.0 && whole.end() > whole.start() {
            let mut expanded = String::new();
            captures.expand(&template(with), &mut expanded);
            return Ok(Some(to_utf16(&expanded)));
        }
    }
    Ok(None)
}

/// Teksten med alle fund skiftet ud, og hvor mange der var.
pub fn replace_all(text: &[u16], query: &Query, with: &[u16]) -> Result<(Vec<u16>, usize), String> {
    let found = matches(text, query)?;
    if query.regex {
        let regex = compile(query)?;
        let utf8 = to_utf8(text);
        let template = template(with);
        let mut out = String::with_capacity(utf8.len());
        let mut units = Units::new(&utf8);
        let mut at = 0;
        let mut count = 0;
        for captures in regex.captures_iter(&utf8) {
            let whole = captures.get(0).expect("gruppe 0 findes altid");
            // Kun de fund der også tæller i søgningen (ikke tomme, og hele ord hvis valgt).
            let start = units.at(whole.start());
            if found.binary_search(&(start, units.at(whole.end()))).is_err() {
                continue;
            }
            out.push_str(&utf8[at..whole.start()]);
            captures.expand(&template, &mut out);
            at = whole.end();
            count += 1;
        }
        out.push_str(&utf8[at..]);
        return Ok((to_utf16(&out), count));
    }
    let mut out = Vec::with_capacity(text.len());
    let mut at = 0;
    for &(start, end) in &found {
        out.extend_from_slice(&text[at..start]);
        out.extend_from_slice(with);
        at = end;
    }
    out.extend_from_slice(&text[at..]);
    Ok((out, found.len()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn w(s: &str) -> Vec<u16> {
        s.replace('\n', "\r").encode_utf16().collect()
    }

    fn plain(text: &str, needle: &str, match_case: bool) -> Vec<usize> {
        let needle = w(needle);
        let query = Query { needle: &needle, match_case, regex: false, whole_word: false };
        matches(&w(text), &query).unwrap().into_iter().map(|(start, _)| start).collect()
    }

    fn regex(text: &str, pattern: &str) -> Result<Vec<Match>, String> {
        let needle = w(pattern);
        matches(&w(text), &Query { needle: &needle, match_case: false, regex: true, whole_word: false })
    }

    fn replaced(text: &str, needle: &str, with: &str, regex: bool) -> (String, usize) {
        let needle = w(needle);
        let query = Query { needle: &needle, match_case: true, regex, whole_word: false };
        let (out, count) = replace_all(&w(text), &query, &w(with)).unwrap();
        (String::from_utf16(&out).unwrap().replace('\r', "\n"), count)
    }

    #[test]
    fn finds_every_occurrence_without_overlap() {
        assert_eq!(plain("en to en tre en", "en", true), [0, 6, 13]);
        assert_eq!(plain("aaaa", "aa", true), [0, 2]);
        assert_eq!(plain("abc", "", true), [] as [usize; 0]);
        assert_eq!(plain("ab", "abc", true), [] as [usize; 0]);
        assert_eq!(plain("abc", "abc", true), [0]);
    }

    #[test]
    fn case_is_ignored_unless_asked_for() {
        assert_eq!(plain("Port PORT port", "port", false), [0, 5, 10]);
        assert_eq!(plain("Port PORT port", "port", true), [10]);
        // Også for danske bogstaver.
        assert_eq!(plain("Æble æble ØL", "æBLE", false), [0, 5]);
        assert_eq!(plain("Æble æble ØL", "øl", false), [10]);
    }

    #[test]
    fn positions_count_utf16_units_like_the_text_field() {
        // En emoji fylder to enheder, og linjeskift er ét tegn (CR).
        assert_eq!(plain("😀 x\nx", "x", true), [3, 5]);
        assert_eq!(regex("😀 æx\nx1", r"x\d?").unwrap(), [(4, 5), (6, 8)]);
    }

    #[test]
    fn plain_search_takes_regex_characters_literally() {
        assert_eq!(plain("a.b a*b (x)", ".", true), [1]);
        assert_eq!(plain("a.b a*b (x)", "(x)", true), [8]);
    }

    #[test]
    fn regex_finds_patterns_of_varying_length() {
        assert_eq!(regex("PORT=8080\nDEBUG=1\nNAME=x", r"\d+").unwrap(), [(5, 9), (16, 17)]);
        assert_eq!(regex("foo Foo FOO", "foo").unwrap(), [(0, 3), (4, 7), (8, 11)]);
        assert_eq!(regex("cat cut cot", "c[au]t").unwrap(), [(0, 3), (4, 7)]);
        assert_eq!(regex("a|b", r"\|").unwrap(), [(1, 2)]);
    }

    #[test]
    fn regex_anchors_apply_to_each_line() {
        // ^ og $ er linjens start og slut, og . går ikke hen over et linjeskift.
        assert_eq!(regex("A=1\nB=2\nC=3", r"^\w").unwrap(), [(0, 1), (4, 5), (8, 9)]);
        assert_eq!(regex("A=1\nB=2\nC=3", r"\d$").unwrap(), [(2, 3), (6, 7), (10, 11)]);
        assert_eq!(regex("ab\ncd", "a.*").unwrap(), [(0, 2)]);
        assert_eq!(regex("ab\ncd", r"b\nc").unwrap(), [(1, 4)]);
    }

    #[test]
    fn regex_ignores_empty_matches_and_reports_bad_patterns() {
        assert_eq!(regex("abc", "^").unwrap(), []);
        assert_eq!(regex("abc", "x*").unwrap(), []);
        assert_eq!(regex("abc", "(").unwrap_err(), "Ugyldigt udtryk");
        assert_eq!(regex("abc", "[a").unwrap_err(), "Ugyldigt udtryk");
    }

    fn whole(text: &str, needle: &str, regex: bool) -> Vec<Match> {
        let needle = w(needle);
        matches(&w(text), &Query { needle: &needle, match_case: false, regex, whole_word: true }).unwrap()
    }

    #[test]
    fn whole_word_skips_matches_inside_longer_words() {
        // "port" står alene to gange; i "export", "ports" og "port_x" er det en del af et ord.
        assert_eq!(whole("port export ports port_x PORT=1", "port", false), [(0, 4), (25, 29)]);
        // Tegnsætning og linjeskift skiller ord ad, også ved tekstens ender.
        assert_eq!(whole("(port)\nport.x,port", "port", false), [(1, 5), (7, 11), (14, 18)]);
        // Danske bogstaver og tal er ordtegn.
        assert_eq!(whole("ø søen ø1 ø", "ø", false), [(0, 1), (10, 11)]);
    }

    #[test]
    fn whole_word_also_applies_to_regex_and_replace_all() {
        assert_eq!(whole("id user_id id2 id", r"i\w", true), [(0, 2), (15, 17)]);
        let needle = w("id");
        let query = Query { needle: &needle, match_case: true, regex: false, whole_word: true };
        let (out, count) = replace_all(&w("id user_id id"), &query, &w("nr")).unwrap();
        assert_eq!((String::from_utf16(&out).unwrap().as_str(), count), ("nr user_id nr", 2));
        let query = Query { regex: true, ..query };
        let (out, count) = replace_all(&w("id user_id id"), &query, &w("<$0>")).unwrap();
        assert_eq!((String::from_utf16(&out).unwrap().as_str(), count), ("<id> user_id <id>", 2));
        // Et fund midt i et ord kan heller ikke erstattes enkeltvis.
        assert_eq!(replacement(&w("id user_id"), &query, (8, 10), &w("x")).unwrap(), None);
    }

    #[test]
    fn next_and_previous_wrap_around() {
        let found = [(2, 5), (10, 13), (20, 23)];
        // Mens man skriver: fundet ved markøren eller det første efter.
        assert_eq!(pick(&found, 0, 0, 0), Some(0));
        assert_eq!(pick(&found, 10, 12, 0), Some(1));
        assert_eq!(pick(&found, 21, 21, 0), Some(0));
        // Næste: fra et fund går man videre; fra et andet sted tager man det første efter.
        assert_eq!(pick(&found, 10, 13, 1), Some(2));
        assert_eq!(pick(&found, 10, 10, 1), Some(1));
        assert_eq!(pick(&found, 20, 23, 1), Some(0));
        // Forrige.
        assert_eq!(pick(&found, 10, 13, -1), Some(0));
        assert_eq!(pick(&found, 2, 5, -1), Some(2));
        assert_eq!(pick(&[], 0, 0, 1), None);
    }

    #[test]
    fn replace_all_swaps_every_occurrence() {
        assert_eq!(replaced("a=1\nb=1\nc=2", "=1", ": en", false), ("a: en\nb: en\nc=2".to_string(), 2));
        // Erstatningen må gerne indeholde det der søges efter, uden at det går i ring.
        assert_eq!(replaced("xx", "x", "xx", false), ("xxxx".to_string(), 2));
        // Og den må være tom.
        assert_eq!(replaced("A-b-C", "-", "", false), ("AbC".to_string(), 2));
        assert_eq!(replaced("abc", "x", "y", false), ("abc".to_string(), 0));
        // Uden regulære udtryk er $1 bare tekst.
        assert_eq!(replaced("a b", " ", "$1", false), ("a$1b".to_string(), 1));
    }

    #[test]
    fn regex_replacement_can_use_groups() {
        assert_eq!(replaced("PORT=8080\nNAME=x", r"(\w+)=(\w+)", "$2 <- $1", true), ("8080 <- PORT\nx <- NAME".to_string(), 2));
        assert_eq!(replaced("a,b", ",", r"\n", true), ("a\nb".to_string(), 1));
        assert_eq!(replaced("a b", " ", r"\t", true), ("a\tb".to_string(), 1));
        assert_eq!(replaced("abc", "b", "[$0]", true), ("a[b]c".to_string(), 1));
    }

    #[test]
    fn single_replacement_only_applies_to_a_real_match() {
        let text = w("PORT=8080 HOST=x");
        let needle = w(r"(\w+)=(\w+)");
        let query = Query { needle: &needle, match_case: true, regex: true, whole_word: false };
        let out = replacement(&text, &query, (10, 16), &w("$2=$1")).unwrap();
        assert_eq!(out.map(|v| String::from_utf16(&v).unwrap()), Some("x=HOST".to_string()));
        // Markeringen er ikke et fund: intet at erstatte.
        assert_eq!(replacement(&text, &query, (0, 4), &w("x")).unwrap(), None);

        let needle = w("port");
        let query = Query { needle: &needle, match_case: false, regex: false, whole_word: false };
        assert_eq!(replacement(&text, &query, (0, 4), &w("$1")).unwrap(), Some(w("$1")));
    }
}
