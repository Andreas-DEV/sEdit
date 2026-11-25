//! Lægger ikon og manifest ind i exe'en som Windows-ressourcer.
//!
//! GNU-toolchainen har ingen ressource-compiler (windres), så `.rsrc`-sektionen
//! skrives her direkte som en COFF-objektfil, som linkeren tager med.

use std::env;
use std::fs;
use std::path::PathBuf;

use embed_manifest::manifest::ActiveCodePage;
use embed_manifest::{embed_manifest, new_manifest};

const RT_ICON: u32 = 3;
const RT_GROUP_ICON: u32 = 14;
const RT_MANIFEST: u32 = 24;
const LANG_EN_US: u32 = 1033;
/// Ikongruppens id. `ui.rs` indlæser vinduesikonet med samme nummer.
const APP_ICON_ID: u32 = 1;

struct Resource {
    kind: u32,
    id: u32,
    data: Vec<u8>,
}

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=assets/sedit.ico");
    if env::var_os("CARGO_CFG_WINDOWS").is_none() {
        return;
    }

    // Manifestet giver per-skærm DPI og moderne dialoger. Legacy-tegnsæt
    // beholdes, så gamle ANSI-filer læses med systemets egen kodeside.
    let manifest = new_manifest("sEdit").active_code_page(ActiveCodePage::Legacy);

    // (maskintype, relokeringstype for en billed-relativ 32-bit adresse)
    let machine = match env::var("CARGO_CFG_TARGET_ARCH").as_deref() {
        Ok("x86_64") => Some((0x8664u16, 3u16)),
        Ok("x86") => Some((0x014c, 7)),
        Ok("aarch64") => Some((0xaa64, 2)),
        _ => None,
    };
    let gnu = env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("gnu");
    let Some((machine, relocation)) = machine.filter(|_| gnu) else {
        // MSVC-linkeren indlejrer selv manifestet. Ikonet kommer ikke med der;
        // appen falder så tilbage til Windows' standardikon.
        embed_manifest(manifest).expect("kunne ikke indlejre manifest");
        return;
    };

    let icon = fs::read("assets/sedit.ico").expect("assets/sedit.ico mangler (kør scripts/make-icon.ps1)");
    let mut resources = icon_resources(&icon);
    resources.push(Resource { kind: RT_MANIFEST, id: 1, data: manifest.to_string().into_bytes() });

    let (section, relocations) = resource_section(resources);
    let object = coff_object(&section, &relocations, machine, relocation);
    let path = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR")).join("sedit-resources.o");
    fs::write(&path, object).expect("kunne ikke skrive ressourcefilen");
    println!("cargo:rustc-link-arg-bins={}", path.display());
}

fn u16_at(bytes: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([bytes[at], bytes[at + 1]])
}

fn u32_at(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
}

/// Deler en .ico-fil op: ét RT_ICON pr. billede og en RT_GROUP_ICON, der peger på dem.
fn icon_resources(ico: &[u8]) -> Vec<Resource> {
    assert!(ico.len() >= 6 && u16_at(ico, 2) == 1, "assets/sedit.ico er ikke en ikonfil");
    let count = u16_at(ico, 4) as usize;
    let mut resources = Vec::with_capacity(count + 1);
    // Gruppen har samme hoved som filen; hver post er filens post med billedets
    // ressource-id i stedet for dets placering i filen.
    let mut group = ico[..6].to_vec();
    for index in 0..count {
        let entry = &ico[6 + index * 16..6 + (index + 1) * 16];
        let size = u32_at(entry, 8) as usize;
        let offset = u32_at(entry, 12) as usize;
        let id = index as u32 + 1;
        group.extend_from_slice(&entry[..12]);
        group.extend_from_slice(&(id as u16).to_le_bytes());
        resources.push(Resource { kind: RT_ICON, id, data: ico[offset..offset + size].to_vec() });
    }
    resources.push(Resource { kind: RT_GROUP_ICON, id: APP_ICON_ID, data: group });
    resources
}

/// IMAGE_RESOURCE_DIRECTORY med kun id-poster.
fn directory_table(entries: usize) -> [u8; 16] {
    let mut table = [0u8; 16];
    table[14..16].copy_from_slice(&(entries as u16).to_le_bytes());
    table
}

/// IMAGE_RESOURCE_DIRECTORY_ENTRY: peger på en undertabel eller på en datapost.
fn directory_entry(id: u32, offset: u32, subdirectory: bool) -> [u8; 8] {
    let mut entry = [0u8; 8];
    entry[..4].copy_from_slice(&id.to_le_bytes());
    let flag = if subdirectory { 0x8000_0000 } else { 0 };
    entry[4..].copy_from_slice(&(offset | flag).to_le_bytes());
    entry
}

/// Bygger `.rsrc`-sektionens indhold: træet type -> id -> sprog, dataposterne og selve
/// dataene. Returnerer også hvor adresserne står, der skal relokeres af linkeren.
fn resource_section(mut resources: Vec<Resource>) -> (Vec<u8>, Vec<u32>) {
    resources.sort_by_key(|r| (r.kind, r.id));
    let mut kinds: Vec<u32> = resources.iter().map(|r| r.kind).collect();
    kinds.dedup();
    let count = resources.len();
    let per_kind = |kind: u32| resources.iter().filter(|r| r.kind == kind).count();

    let id_tables_start = 16 + 8 * kinds.len();
    let language_tables_start = id_tables_start + kinds.iter().map(|&k| 16 + 8 * per_kind(k)).sum::<usize>();
    let data_entries_start = language_tables_start + 24 * count;
    let data_start = data_entries_start + 16 * count;

    let mut data_offsets = Vec::with_capacity(count);
    let mut end = data_start;
    for r in &resources {
        data_offsets.push(end);
        end += (r.data.len() + 7) & !7;
    }

    let mut out = Vec::with_capacity(end);
    out.extend_from_slice(&directory_table(kinds.len()));
    let mut table_at = id_tables_start;
    for &kind in &kinds {
        out.extend_from_slice(&directory_entry(kind, table_at as u32, true));
        table_at += 16 + 8 * per_kind(kind);
    }
    for &kind in &kinds {
        out.extend_from_slice(&directory_table(per_kind(kind)));
        for (index, r) in resources.iter().enumerate().filter(|(_, r)| r.kind == kind) {
            out.extend_from_slice(&directory_entry(r.id, (language_tables_start + 24 * index) as u32, true));
        }
    }
    for index in 0..count {
        out.extend_from_slice(&directory_table(1));
        out.extend_from_slice(&directory_entry(LANG_EN_US, (data_entries_start + 16 * index) as u32, false));
    }
    let mut relocations = Vec::with_capacity(count);
    for (index, r) in resources.iter().enumerate() {
        // IMAGE_RESOURCE_DATA_ENTRY: adresse (relokeres), størrelse, kodeside, reserveret.
        relocations.push(out.len() as u32);
        out.extend_from_slice(&(data_offsets[index] as u32).to_le_bytes());
        out.extend_from_slice(&(r.data.len() as u32).to_le_bytes());
        out.extend_from_slice(&[0; 8]);
    }
    for r in &resources {
        out.extend_from_slice(&r.data);
        out.resize((out.len() + 7) & !7, 0);
    }
    assert_eq!(out.len(), end);
    (out, relocations)
}

/// En COFF-objektfil med én `.rsrc`-sektion. Tidsstemplet er nul, så bygget er reproducerbart.
fn coff_object(section: &[u8], relocations: &[u32], machine: u16, relocation_type: u16) -> Vec<u8> {
    const HEADERS: u32 = 20 + 40;
    let size = section.len() as u32;
    let relocation_count = relocations.len() as u16;
    let symbol_table = HEADERS + size + 10 * relocations.len() as u32;

    let mut out = Vec::with_capacity(symbol_table as usize + 40);
    // Filhoved.
    out.extend_from_slice(&machine.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes()); // antal sektioner
    out.extend_from_slice(&0u32.to_le_bytes()); // tidsstempel
    out.extend_from_slice(&symbol_table.to_le_bytes());
    out.extend_from_slice(&2u32.to_le_bytes()); // symboler: sektionen plus dens hjælpepost
    out.extend_from_slice(&[0; 4]); // intet valgfrit hoved, ingen flag

    // Sektionshoved.
    out.extend_from_slice(b".rsrc\0\0\0");
    out.extend_from_slice(&[0; 8]); // virtuel størrelse og adresse
    out.extend_from_slice(&size.to_le_bytes());
    out.extend_from_slice(&HEADERS.to_le_bytes()); // her starter dataene
    out.extend_from_slice(&(HEADERS + size).to_le_bytes()); // her starter relokeringerne
    out.extend_from_slice(&[0; 4]);
    out.extend_from_slice(&relocation_count.to_le_bytes());
    out.extend_from_slice(&[0; 2]);
    out.extend_from_slice(&0xC030_0040u32.to_le_bytes()); // initialiserede data, 4-byte justering, læs/skriv

    out.extend_from_slice(section);
    for &address in relocations {
        out.extend_from_slice(&address.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes()); // symbol 0 = sektionen selv
        out.extend_from_slice(&relocation_type.to_le_bytes());
    }

    // Symboltabel: sektionssymbolet og dets hjælpepost, derefter en tom strengtabel.
    out.extend_from_slice(b".rsrc\0\0\0");
    out.extend_from_slice(&[0; 4]);
    out.extend_from_slice(&1u16.to_le_bytes()); // sektion nummer 1
    out.extend_from_slice(&[0, 0, 3, 1]); // type 0, statisk, én hjælpepost
    out.extend_from_slice(&size.to_le_bytes());
    out.extend_from_slice(&relocation_count.to_le_bytes());
    out.extend_from_slice(&[0; 12]);
    out.extend_from_slice(&4u32.to_le_bytes());
    out
}
