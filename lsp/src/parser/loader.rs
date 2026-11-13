//! File discovery and text decoding for an Emuera game directory.
//!
//! Emuera games ship ERB/ERH/CSV files either as UTF-8 (usually with BOM)
//! or as Shift_JIS. We decode both transparently.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// Read a text file, auto-detecting UTF-8 (with/without BOM) vs Shift_JIS.
pub fn read_text(path: &Path) -> io::Result<String> {
    let bytes = fs::read(path)?;
    Ok(decode(&bytes))
}

/// `s`'s lines as Emuera reads them (C#'s ReadAllLines): a line ends at
/// "\r\n", "\n" or a lone "\r" (eraMaouEX has `\rDRAWLINE\rCALL ...`).
pub fn emuera_lines(s: &str) -> impl Iterator<Item = &str> {
    let mut rest = s;
    let lone = has_lone_cr(s);
    std::iter::from_fn(move || {
        if rest.is_empty() {
            return None;
        }
        let nl = rest.find('\n');
        let seg = &rest[..nl.unwrap_or(rest.len())];
        if !lone {
            rest = nl.map_or("", |i| &rest[i + 1..]);
            return Some(seg.strip_suffix('\r').unwrap_or(seg));
        }
        Some(match seg.find('\r') {
            Some(j) if j + 1 == seg.len() && nl.is_some() => {
                rest = &rest[j + 2..];
                &seg[..j]
            }
            Some(j) => {
                rest = &rest[j + 1..];
                &seg[..j]
            }
            None => {
                rest = nl.map_or("", |i| &rest[i + 1..]);
                seg
            }
        })
    })
}

/// Has `s` a "\r" that isn't the start of a "\r\n"?
pub fn has_lone_cr(s: &str) -> bool {
    s.match_indices('\r')
        .any(|(i, _)| s.as_bytes().get(i + 1) != Some(&b'\n'))
}

pub fn decode(bytes: &[u8]) -> String {
    decode_detect(bytes).0
}

/// [`decode`], and whether the bytes were read as Shift_JIS (one pass over
/// them: a big game has 50 MB of scripts).
pub fn decode_detect(bytes: &[u8]) -> (String, bool) {
    if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
        return (String::from_utf8_lossy(&bytes[3..]).into_owned(), false);
    }
    match std::str::from_utf8(bytes) {
        Ok(s) => (s.to_owned(), false),
        Err(_) => {
            let (cow, _, _) = encoding_rs::SHIFT_JIS.decode(bytes);
            (cow.into_owned(), true)
        }
    }
}

/// Whether [`decode`] reads these bytes as Shift_JIS.
pub fn is_sjis(bytes: &[u8]) -> bool {
    !bytes.starts_with(&[0xEF, 0xBB, 0xBF]) && std::str::from_utf8(bytes).is_err()
}

/// Part of a file (whole lines) in the encoding [`decode`] found for it.
pub fn decode_as(bytes: &[u8], sjis: bool) -> String {
    if sjis {
        return encoding_rs::SHIFT_JIS.decode(bytes).0.into_owned();
    }
    String::from_utf8_lossy(bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes)).into_owned()
}

/// Find a direct child directory by case-insensitive name.
pub fn find_dir_ci(base: &Path, name: &str) -> Option<PathBuf> {
    let rd = fs::read_dir(base).ok()?;
    for e in rd.flatten() {
        if e.file_name().to_string_lossy().eq_ignore_ascii_case(name) && e.path().is_dir() {
            return Some(e.path());
        }
    }
    None
}

/// Find a file in `dir` by case-insensitive name.
pub fn find_file_ci(dir: &Path, name: &str) -> Option<PathBuf> {
    let rd = fs::read_dir(dir).ok()?;
    for e in rd.flatten() {
        if e.file_name().to_string_lossy().eq_ignore_ascii_case(name) {
            return Some(e.path());
        }
    }
    None
}

/// How Emuera lists the ERB folder: `サブディレクトリを検索する` (look in
/// subfolders) and `読み込み順をファイル名順にソートする` (sort by name),
/// both off unless the config turns them on.
#[derive(Clone, Copy, Debug)]
pub struct Listing {
    pub subdirs: bool,
    pub order: Order,
}

/// How a folder's names are ordered: unsorted (as Windows lists NTFS), or
/// sorted by .NET's culture order, which is ICU's on .NET Core (Emuera-EM /
/// EE) and Windows' NLS on .NET Framework (Emuera 1821, Anchor). The two
/// disagree on punctuation, so on which of `A.ERB` and `A_B.ERB` loads
/// first and wins a function both define.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Order {
    Ntfs,
    Icu,
    Nls,
}

impl Listing {
    pub fn of(game: &Path, subdirs: bool, order: Order) -> Listing {
        Listing { subdirs, order }
    }
}

/// Whether the game ships its Emuera and only .NET Framework builds of it
/// (no `.NETCoreApp` in any of its .exe files): the order its scripts were
/// written for is then NLS's.
fn framework_emuera(game: &Path) -> bool {
    let exes: Vec<PathBuf> = fs::read_dir(game)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_file() && p.extension().is_some_and(|x| x.eq_ignore_ascii_case("exe")))
        .collect();
    !exes.is_empty()
        && !exes
            .iter()
            .any(|p| fs::read(p).is_ok_and(|b| b.windows(12).any(|w| w == b".NETCoreApp,")))
}

/// Emuera's order of names in a folder (see [`Order`]).
pub fn name_order(a: &str, b: &str, order: Order) -> std::cmp::Ordering {
    if order == Order::Ntfs {
        let up = |s: &str| {
            s.encode_utf16()
                .map(|u| {
                    char::from_u32(u as u32).map_or(u as u32, |c| {
                        c.to_uppercase().next().map_or(u as u32, |c| c as u32)
                    })
                })
                .collect::<Vec<u32>>()
        };
        return up(a).cmp(&up(b)).then_with(|| a.cmp(b));
    }
    let punct = match order {
        Order::Nls => "!\"#$%&()*,./:;?@[\\]^_`{|}~+<=>",
        _ => "_-,;:!?.'\"()[]{}@*/\\&#%`^+<=>|~$",
    };
    let key = |c: char| -> (u8, u32) {
        let c = match c as u32 {
            0xFF01..=0xFF5E => char::from_u32(c as u32 - 0xFEE0).unwrap_or(c),
            0x3000 => ' ',
            _ => c,
        };
        match c {
            ' ' | '\t' => (0, 0),
            '0'..='9' => (1, c as u32),
            'a'..='z' | 'A'..='Z' => (2, c.to_ascii_lowercase() as u32),
            _ if c.is_ascii() => (0, punct.find(c).map_or(200, |i| i as u32 + 1)),
            '\u{2000}'..='\u{2BFF}' | '\u{3001}'..='\u{303F}' | '\u{30FB}' => (0, 1000 + c as u32),
            '\u{30A1}'..='\u{30F6}' => (3, c as u32 - 0x60),
            _ => (3, c as u32),
        }
    };
    let skip = |c: &char| order == Order::Nls && (*c == '-' || *c == '\'');
    let ka: Vec<(u8, u32)> = a.chars().filter(|c| !skip(c)).map(key).collect();
    let kb: Vec<(u8, u32)> = b.chars().filter(|c| !skip(c)).map(key).collect();
    let case = |s: &str| {
        s.chars()
            .map(|c| c.is_uppercase() as u8)
            .collect::<Vec<u8>>()
    };
    ka.cmp(&kb)
        .then_with(|| case(a).cmp(&case(b)))
        .then_with(|| a.cmp(b))
}

/// Emuera's `Config.GetFiles`: the files of `dir` with one of the given
/// (lowercase) extensions, then (with `subdirs`) each subfolder's the
/// same way, each level in `name_order`.
pub fn emuera_files(dir: &Path, exts: &[&str], l: Listing) -> Vec<PathBuf> {
    let mut out = Vec::new();
    list_into(dir, exts, l, &mut out);
    out
}

fn list_into(dir: &Path, exts: &[&str], l: Listing, out: &mut Vec<PathBuf>) {
    let Ok(rd) = fs::read_dir(dir) else { return };
    let (mut files, mut dirs) = (Vec::new(), Vec::new());
    for e in rd.flatten() {
        let p = e.path();
        let name = e.file_name().to_string_lossy().into_owned();
        if p.is_dir() {
            dirs.push((name, p));
        } else if let Some(ext) = p.extension() {
            let ext = ext.to_string_lossy().to_lowercase();
            if exts.iter().any(|x| *x == ext) {
                files.push((name, p));
            }
        }
    }
    files.sort_by(|a, b| name_order(&a.0, &b.0, l.order));
    out.extend(files.into_iter().map(|f| f.1));
    if l.subdirs {
        dirs.sort_by(|a, b| name_order(&a.0, &b.0, l.order));
        for (_, d) in dirs {
            list_into(&d, exts, l, out);
        }
    }
}

/// The ERB files in Emuera's load order: first those under folders whose
/// name has a `#` (Emuera-EE's load-first folders), then the rest.
pub fn erb_files(erb: &Path, l: Listing) -> Vec<PathBuf> {
    fn hash_dirs(d: &Path, deep: bool, sort: Order, out: &mut Vec<PathBuf>) {
        let Ok(rd) = fs::read_dir(d) else { return };
        let mut dirs: Vec<(String, PathBuf)> = rd
            .flatten()
            .filter(|e| e.path().is_dir())
            .map(|e| (e.file_name().to_string_lossy().into_owned(), e.path()))
            .collect();
        dirs.sort_by(|a, b| name_order(&a.0, &b.0, sort));
        for (name, p) in dirs {
            if name.contains('#') {
                out.push(p.clone());
            }
            if deep {
                hash_dirs(&p, deep, sort, out);
            }
        }
    }
    let mut first = Vec::new();
    hash_dirs(erb, l.subdirs, l.order, &mut first);
    let mut out: Vec<PathBuf> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for d in first {
        for f in emuera_files(&d, &["erb"], l) {
            if seen.insert(f.clone()) {
                out.push(f);
            }
        }
    }
    for f in emuera_files(erb, &["erb"], l) {
        if seen.insert(f.clone()) {
            out.push(f);
        }
    }
    out
}

/// Recursively collect files with one of the given (lowercase) extensions,
/// sorted case-insensitively by path.
pub fn walk(dir: &Path, exts: &[&str]) -> Vec<PathBuf> {
    let mut out = Vec::new();
    walk_inner(dir, exts, &mut out);
    out.sort_by_key(|p| p.to_string_lossy().to_lowercase());
    out
}

fn walk_inner(dir: &Path, exts: &[&str], out: &mut Vec<PathBuf>) {
    let Ok(rd) = fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            walk_inner(&p, exts, out);
        } else if let Some(ext) = p.extension() {
            let ext = ext.to_string_lossy().to_lowercase();
            if exts.iter().any(|x| *x == ext) {
                out.push(p);
            }
        }
    }
}

/// Split a CSV line Emuera-style: comma separated, `;` starts a comment,
/// fields are trimmed. Empty trailing fields are kept.
pub fn csv_fields(line: &str) -> Vec<String> {
    let line = match line.find(';') {
        Some(i) => &line[..i],
        None => line,
    };
    if line.trim().is_empty() {
        return Vec::new();
    }
    line.split(',').map(|s| s.trim().to_string()).collect()
}

/// 内部で使用する東アジア言語 (Emuera's useLanguage): the code page byte
/// lengths are counted in. 0 Japanese (932), 1 Korean (949), 2 Simplified
/// Chinese (936), 3 Traditional Chinese (950).
pub static ANSI_LANGUAGE: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);

pub fn ansi_language(name: &str) -> u8 {
    match name.trim().to_uppercase().as_str() {
        "KOREAN" => 1,
        "CHINESE_HANS" => 2,
        "CHINESE_HANT" => 3,
        _ => 0,
    }
}

/// Byte length in the game's code page.
pub fn sjis_len(s: &str) -> usize {
    let lang = match ANSI_LANGUAGE.load(std::sync::atomic::Ordering::Relaxed) {
        1 => Some(encoding_rs::EUC_KR),
        2 => Some(encoding_rs::GBK),
        3 => Some(encoding_rs::BIG5),
        _ => None,
    };
    s.chars()
        .map(|c| {
            if c.is_ascii() {
                return 1;
            }
            let mut buf = [0u8; 4];
            let enc = c.encode_utf8(&mut buf);
            if let Some(l) = lang {
                let (bytes, _, unmappable) = l.encode(enc);
                if !unmappable {
                    return bytes.len();
                }
            }
            let (bytes, _, unmappable) = encoding_rs::SHIFT_JIS.encode(enc);
            if unmappable {
                c.len_utf16()
            } else {
                bytes.len()
            }
        })
        .sum()
}

/// Check if a system variable name is a string type
pub fn is_system_str_var(name: &str) -> bool {
    matches!(
        name,
        "RESULT"
            | "RESULTSTR"
            | "STR"
            | "STRS"
            | "LOCALS"
            | "ARGS"
            | "THIS"
            | "TARGET"
            | "CALLER"
            | "ME"
            | "YOU"
            | "PARTNER"
            | "MUSIC"
            | "BGM"
            | "SOUND"
            | "SE"
            | "VOICE"
            | "SAVE"
            | "LOAD"
            | "DATA"
            | "GLOBAL"
            | "GLOBALS"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_emuera_lines() {
        let v: Vec<&str> = emuera_lines("a\r\nb\rc\n\rd\n").collect();
        assert_eq!(v, ["a", "b", "c", "", "d"]);
        assert_eq!(emuera_lines("x").collect::<Vec<_>>(), ["x"]);
        assert_eq!(emuera_lines("").count(), 0);
        let v: Vec<&str> = emuera_lines("a\r\nb\nc\r\n").collect();
        assert_eq!(v, ["a", "b", "c"]);
        assert!(has_lone_cr("a\rb") && has_lone_cr("a\r") && !has_lone_cr("a\r\nb"));
    }

    #[test]
    fn test_sjis_len() {
        assert_eq!(sjis_len("abc"), 3);
        assert_eq!(sjis_len("あい"), 4);
        assert_eq!(sjis_len("ｱｲ"), 2);
        assert_eq!(sjis_len("♂"), 2);
        assert_eq!(sjis_len("한"), 1);
        assert_eq!(sjis_len("😀"), 2);
    }
}
