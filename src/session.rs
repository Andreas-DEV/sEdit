//! Sessionen: hvilke faner der var åbne, da sEdit lukkede, og den ugemte tekst i dem.
//!
//! Den ligger i en mappe med `index.txt` (én linje pr. fane) og en kladdefil pr. fane
//! med ugemte ændringer. Kladden er tekstfeltets indhold som rå UTF-16.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::doc::{Enc, Eol, Sig};

const HEADER: &str = "sEdit-session 1";

/// Alt hvad der skal til for at genskabe en fane.
#[derive(Clone)]
pub struct Snapshot {
    pub path: Option<PathBuf>,
    /// Fanens navn; bruges for faner uden fil ("Unavngivet 2").
    pub name: String,
    pub enc: Enc,
    pub eol: Eol,
    /// Tekstfeltets indhold, hvis det afveg fra filen. None for en fane uden ugemte ændringer.
    pub draft: Option<Vec<u16>>,
    /// Fingeraftryk af filens indhold, som kladden bygger på. Passer det ikke længere
    /// med filen ved gendannelse, er filen ændret i mellemtiden.
    pub base: Sig,
    pub selection: (i32, i32),
    pub active: bool,
}

fn enc_name(enc: Enc) -> &'static str {
    match enc {
        Enc::Utf8 => "utf8",
        Enc::Utf8Bom => "utf8bom",
        Enc::Utf16Le => "utf16le",
        Enc::Utf16Be => "utf16be",
        Enc::Ansi => "ansi",
    }
}

fn enc_from(name: &str) -> Enc {
    match name {
        "utf8bom" => Enc::Utf8Bom,
        "utf16le" => Enc::Utf16Le,
        "utf16be" => Enc::Utf16Be,
        "ansi" => Enc::Ansi,
        _ => Enc::Utf8,
    }
}

fn eol_name(eol: Eol) -> &'static str {
    match eol {
        Eol::Crlf => "crlf",
        Eol::Lf => "lf",
        Eol::Cr => "cr",
    }
}

fn eol_from(name: &str) -> Eol {
    match name {
        "lf" => Eol::Lf,
        "cr" => Eol::Cr,
        _ => Eol::Crlf,
    }
}

/// Skriver sessionen. Indekset skrives til sidst og byttes ind i ét hug, så en
/// afbrydelse undervejs efterlader den forrige session i stedet for en halv.
pub fn save(dir: &Path, tabs: &[Snapshot]) -> io::Result<()> {
    fs::create_dir_all(dir)?;
    // Kladderne får et nyt løbenummer hver gang, så de gamle først kan ryddes væk,
    // når det nye indeks peger på de nye.
    let generation = fs::read_to_string(dir.join("generation")).ok().and_then(|s| s.trim().parse::<u64>().ok()).unwrap_or(0) + 1;

    let mut index = String::from(HEADER);
    index.push('\n');
    for (number, tab) in tabs.iter().enumerate() {
        let draft_file = match &tab.draft {
            Some(text) => {
                let name = format!("{generation}-{number}.draft");
                let bytes: Vec<u8> = text.iter().flat_map(|unit| unit.to_le_bytes()).collect();
                fs::write(dir.join(&name), bytes)?;
                name
            }
            None => "-".to_string(),
        };
        // Stien står sidst, så den må indeholde hvad som helst undtagen linjeskift.
        let path = tab.path.as_ref().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
        index.push_str(&format!(
            "{}\t{}\t{}\t{}\t{}\t{:016x}\t{}\t{}\t{}\t{}\n",
            if tab.active { "A" } else { "-" },
            draft_file,
            enc_name(tab.enc),
            eol_name(tab.eol),
            tab.base.len,
            tab.base.hash,
            tab.selection.0,
            tab.selection.1,
            tab.name.replace(['\t', '\n', '\r'], " "),
            path,
        ));
    }

    let temporary = dir.join("index.tmp");
    fs::write(&temporary, index)?;
    fs::rename(&temporary, dir.join("index.txt"))?;
    fs::write(dir.join("generation"), generation.to_string())?;

    // Kladder fra tidligere sessioner er nu forældede.
    let current = format!("{generation}-");
    for entry in fs::read_dir(dir)?.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.ends_with(".draft") && !name.starts_with(&current) {
            let _ = fs::remove_file(entry.path());
        }
    }
    Ok(())
}

/// Læser sessionen. En manglende eller ulæselig session giver bare ingen faner.
pub fn load(dir: &Path) -> Vec<Snapshot> {
    let Ok(index) = fs::read_to_string(dir.join("index.txt")) else { return Vec::new() };
    let mut lines = index.lines();
    if lines.next() != Some(HEADER) {
        return Vec::new();
    }
    lines.filter_map(|line| parse_line(dir, line)).collect()
}

fn parse_line(dir: &Path, line: &str) -> Option<Snapshot> {
    let fields: Vec<&str> = line.splitn(10, '\t').collect();
    let [active, draft_file, enc, eol, len, hash, start, end, name, path] = fields.as_slice() else { return None };
    let draft = if *draft_file == "-" {
        None
    } else {
        // Kladdefilens navn kommer fra indekset; den må ikke kunne pege ud af mappen.
        if draft_file.contains(['/', '\\', ':']) {
            return None;
        }
        let bytes = fs::read(dir.join(draft_file)).ok()?;
        Some(bytes.chunks_exact(2).map(|pair| u16::from_le_bytes([pair[0], pair[1]])).collect())
    };
    Some(Snapshot {
        path: (!path.is_empty()).then(|| PathBuf::from(path)),
        name: name.to_string(),
        enc: enc_from(enc),
        eol: eol_from(eol),
        draft,
        base: Sig { len: len.parse().ok()?, hash: u64::from_str_radix(hash, 16).ok()? },
        selection: (start.parse().unwrap_or(0), end.parse().unwrap_or(0)),
        active: *active == "A",
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("sedit-test-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    fn tab(path: Option<&str>, draft: Option<&str>) -> Snapshot {
        Snapshot {
            path: path.map(PathBuf::from),
            name: "Unavngivet 3".into(),
            enc: Enc::Utf8Bom,
            eol: Eol::Lf,
            draft: draft.map(|s| s.encode_utf16().collect()),
            base: Sig { len: 7, hash: 0xDEAD_BEEF_0123_4567 },
            selection: (4, 9),
            active: false,
        }
    }

    #[test]
    fn session_round_trips_tabs_drafts_and_order() {
        let dir = temp_dir("roundtrip");
        let mut second = tab(None, Some("ugemt æøå\rlinje 2\r"));
        second.active = true;
        save(&dir, &[tab(Some(r"C:\projekt\med mellemrum\.env"), None), second]).unwrap();

        let loaded = load(&dir);
        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded[0].path.as_deref(), Some(Path::new(r"C:\projekt\med mellemrum\.env")));
        assert!(loaded[0].draft.is_none() && !loaded[0].active);
        assert_eq!((loaded[0].enc, loaded[0].eol), (Enc::Utf8Bom, Eol::Lf));
        assert_eq!(loaded[0].base, Sig { len: 7, hash: 0xDEAD_BEEF_0123_4567 });
        assert_eq!(loaded[0].selection, (4, 9));

        assert!(loaded[1].path.is_none() && loaded[1].active);
        assert_eq!(loaded[1].name, "Unavngivet 3");
        assert_eq!(String::from_utf16(loaded[1].draft.as_ref().unwrap()).unwrap(), "ugemt æøå\rlinje 2\r");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn saving_again_replaces_the_session_and_removes_old_drafts() {
        let dir = temp_dir("replace");
        save(&dir, &[tab(Some(r"C:\a.env"), Some("gammel kladde")), tab(Some(r"C:\b.env"), Some("en til"))]).unwrap();
        save(&dir, &[tab(Some(r"C:\c.env"), Some("ny kladde"))]).unwrap();

        let loaded = load(&dir);
        assert_eq!(loaded.len(), 1);
        assert_eq!(String::from_utf16(loaded[0].draft.as_ref().unwrap()).unwrap(), "ny kladde");
        let drafts = fs::read_dir(&dir).unwrap().flatten().filter(|e| e.file_name().to_string_lossy().ends_with(".draft")).count();
        assert_eq!(drafts, 1);

        save(&dir, &[]).unwrap();
        assert!(load(&dir).is_empty());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_or_foreign_session_gives_no_tabs() {
        let dir = temp_dir("missing");
        assert!(load(&dir).is_empty());
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("index.txt"), "noget helt andet\n-\t-\n").unwrap();
        assert!(load(&dir).is_empty());
        // En kladdefil uden for sessionsmappen afvises.
        fs::write(dir.join("index.txt"), format!("{HEADER}\n-\t..\\hemmelig.txt\tutf8\tlf\t0\t0\t0\t0\tx\tC:\\a\n")).unwrap();
        assert!(load(&dir).is_empty());
        let _ = fs::remove_dir_all(&dir);
    }
}
