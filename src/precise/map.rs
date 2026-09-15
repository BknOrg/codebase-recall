//! Translating between what the cache stores (byte offsets, project-relative
//! paths) and what LSP speaks (UTF-16 line/character positions, `file://` URIs).

use std::path::{Path, PathBuf};

use crate::cache::models::SymbolRow;

/// Line index over one file's text, for turning byte offsets into LSP positions.
pub struct PositionMap {
    text: String,
    /// Byte offset where each line starts.
    line_starts: Vec<usize>,
}

impl PositionMap {
    pub fn new(text: String) -> Self {
        let mut line_starts = vec![0usize];
        for (i, b) in text.bytes().enumerate() {
            if b == b'\n' {
                line_starts.push(i + 1);
            }
        }
        Self { text, line_starts }
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    /// Zero-based `(line, character)` for a byte offset, where `character` is a
    /// UTF-16 code-unit count — the unit LSP uses by default.
    ///
    /// Returns `None` when the offset is past the end or lands inside a
    /// multi-byte character, which means the cache and the file on disk have
    /// drifted apart and the position would be meaningless.
    pub fn position(&self, byte: usize) -> Option<(u32, u32)> {
        if byte > self.text.len() {
            return None;
        }
        let line = match self.line_starts.binary_search(&byte) {
            Ok(exact) => exact,
            Err(next) => next - 1,
        };
        let start = self.line_starts[line];
        let prefix = self.text.get(start..byte)?;
        let character = prefix.encode_utf16().count();
        Some((line as u32, character as u32))
    }
}

/// Drop Windows' extended-length prefix, which `canonicalize` adds and which
/// language servers reject when it reaches them inside a `file://` URI.
///
/// `\\?\C:\proj` -> `C:\proj`, `\\?\UNC\host\share` -> `\\host\share`.
pub fn strip_extended_prefix(path: &Path) -> PathBuf {
    let text = path.to_string_lossy();
    if let Some(rest) = text.strip_prefix(r"\\?\UNC\") {
        return PathBuf::from(format!(r"\\{rest}"));
    }
    match text.strip_prefix(r"\\?\") {
        // Only a drive path is safe to shorten; other device paths are not
        // addressable without the prefix.
        Some(rest) if rest.as_bytes().get(1) == Some(&b':') => PathBuf::from(rest),
        _ => path.to_path_buf(),
    }
}

/// `C:\proj\src\main.rs` -> `file:///C:/proj/src/main.rs`. The path must be
/// absolute: a relative one would produce a URI no server can open.
pub fn path_to_uri(path: &Path) -> String {
    let text = path.to_string_lossy().replace('\\', "/");
    let mut out = String::from("file://");
    if !text.starts_with('/') {
        out.push('/'); // Windows drive paths need the extra root slash.
    }
    for ch in text.chars() {
        match ch {
            'A'..='Z' | 'a'..='z' | '0'..='9' | '-' | '.' | '_' | '~' | '/' | ':' => out.push(ch),
            other => {
                let mut buf = [0u8; 4];
                for b in other.encode_utf8(&mut buf).as_bytes() {
                    out.push_str(&format!("%{b:02X}"));
                }
            }
        }
    }
    out
}

/// Inverse of [`path_to_uri`]. Returns `None` for non-`file:` URIs, which is
/// how servers report definitions inside jars, zips or virtual documents.
pub fn uri_to_path(uri: &str) -> Option<PathBuf> {
    let rest = uri.strip_prefix("file://")?;
    // `file:///C:/x` and `file://localhost/C:/x` both mean the local machine.
    let rest = rest.strip_prefix("localhost").unwrap_or(rest);
    let decoded = percent_decode(rest);
    // Strip the root slash in front of a Windows drive letter (`/C:/x`).
    let trimmed = decoded
        .strip_prefix('/')
        .filter(|r| is_windows_drive(r))
        .unwrap_or(&decoded);
    Some(PathBuf::from(trimmed))
}

fn is_windows_drive(s: &str) -> bool {
    let mut it = s.chars();
    matches!(it.next(), Some(c) if c.is_ascii_alphabetic()) && it.next() == Some(':')
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).ok();
            if let Some(b) = hex.and_then(|h| u8::from_str_radix(h, 16).ok()) {
                out.push(b);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The narrowest symbol in `syms` whose line range covers `line` (1-based).
///
/// A definition lands on the line a symbol is declared on, so symbols starting
/// exactly there win over merely enclosing ones — that is what separates
/// `fn bar` from the `impl` block around it.
pub fn symbol_at_line<'a>(syms: &[&'a SymbolRow], line: i64) -> Option<&'a SymbolRow> {
    let mut best: Option<(bool, i64, &SymbolRow)> = None;
    for s in syms {
        let (Some(start), Some(end)) = (s.start_line, s.end_line) else {
            continue;
        };
        if line < start || line > end {
            continue;
        }
        let key = (start == line, -(end - start));
        if best.is_none_or(|(exact, width, _)| (key.0, key.1) > (exact, width)) {
            best = Some((key.0, key.1, s));
        }
    }
    best.map(|(_, _, s)| s)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sym(id: i64, start: i64, end: i64) -> SymbolRow {
        SymbolRow {
            id,
            file_id: 1,
            name: format!("s{id}"),
            kind: "function".into(),
            parent_symbol_id: None,
            is_exported: true,
            start_line: Some(start),
            end_line: Some(end),
            start_byte: None,
            end_byte: None,
            signature: None,
            param_count: None,
            type_name: None,
        }
    }

    #[test]
    fn positions_count_utf16_units() {
        let m = PositionMap::new("fn a() {}\nlet x = \"€\"; y()\n".to_string());
        assert_eq!(m.position(0), Some((0, 0)));
        assert_eq!(m.position(3), Some((0, 3)));
        // Start of line 2.
        assert_eq!(m.position(10), Some((1, 0)));
        // `€` is 3 bytes but a single UTF-16 unit: the call after it sits at
        // character 13, not 15.
        let byte = m.text().find("y()").unwrap();
        assert_eq!(m.position(byte), Some((1, 13)));
        assert_eq!(m.position(9_999), None);
    }

    #[test]
    fn extended_length_prefixes_are_dropped() {
        assert_eq!(
            strip_extended_prefix(Path::new(r"\\?\C:\proj\src")),
            PathBuf::from(r"C:\proj\src")
        );
        assert_eq!(
            strip_extended_prefix(Path::new(r"\\?\UNC\host\share\proj")),
            PathBuf::from(r"\\host\share\proj")
        );
        // Already plain, and non-drive device paths, are left alone.
        assert_eq!(
            strip_extended_prefix(Path::new("/home/u/proj")),
            PathBuf::from("/home/u/proj")
        );
        assert_eq!(
            strip_extended_prefix(Path::new(r"\\?\Volume{abc}\x")),
            PathBuf::from(r"\\?\Volume{abc}\x")
        );
    }

    #[test]
    fn uri_round_trips_windows_and_unix_paths() {
        let win = Path::new(r"C:\proj\src\my file.rs");
        let uri = path_to_uri(win);
        assert_eq!(uri, "file:///C:/proj/src/my%20file.rs");
        assert_eq!(
            uri_to_path(&uri).unwrap().to_string_lossy().replace('\\', "/"),
            "C:/proj/src/my file.rs"
        );

        let unix = Path::new("/home/u/proj/main.rs");
        let uri = path_to_uri(unix);
        assert_eq!(uri, "file:///home/u/proj/main.rs");
        assert_eq!(uri_to_path(&uri).unwrap(), unix);

        // Definitions inside a jar are not files we can map to a node.
        assert!(uri_to_path("jdt://contents/rt.jar/java.lang/String.class").is_none());
    }

    #[test]
    fn innermost_symbol_wins_and_declaration_line_beats_enclosure() {
        let impl_block = sym(1, 10, 30);
        let method = sym(2, 12, 14);
        let syms = vec![&impl_block, &method];

        assert_eq!(symbol_at_line(&syms, 12).map(|s| s.id), Some(2));
        assert_eq!(symbol_at_line(&syms, 20).map(|s| s.id), Some(1));
        assert!(symbol_at_line(&syms, 99).is_none());
    }
}
