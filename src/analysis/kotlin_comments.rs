//! Work around the grammar's block-comment ambiguity without changing source offsets.
//! Only complete comments outside strings, chars and escaped identifiers are masked.
pub fn mask(source: &str) -> String {
    let b = source.as_bytes();
    let mut out = b.to_vec();
    let mut i = 0;
    while i < b.len() {
        if b[i..].starts_with(b"//") {
            while i < b.len() && b[i] != b'\n' {
                i += 1;
            }
        } else if b[i..].starts_with(b"/*") {
            let start = i;
            i += 2;
            let mut depth = 1;
            while i < b.len() && depth > 0 {
                if b[i..].starts_with(b"/*") {
                    depth += 1;
                    i += 2;
                } else if b[i..].starts_with(b"*/") {
                    depth -= 1;
                    i += 2;
                } else {
                    i += 1;
                }
            }
            if depth == 0 {
                for ch in &mut out[start..i] {
                    if !matches!(*ch, b'\n' | b'\r') {
                        *ch = b' ';
                    }
                }
            }
        } else if b[i..].starts_with(b"\"\"\"") {
            i += 3;
            while i < b.len() && !b[i..].starts_with(b"\"\"\"") {
                i += 1;
            }
            i = (i + 3).min(b.len());
        } else if matches!(b[i], b'"' | b'\'' | b'`') {
            let quote = b[i];
            i += 1;
            while i < b.len() {
                if b[i] == quote {
                    i += 1;
                    break;
                }
                if b[i] == b'\\' && quote != b'`' {
                    i = (i + 2).min(b.len());
                } else {
                    i += 1;
                }
            }
        } else {
            i += 1;
        }
    }
    String::from_utf8(out).expect("mask preserves UTF-8")
}

#[cfg(test)]
mod tests {
    #[test]
    fn comments_masked_but_strings_locations_and_unterminated_comments_survive() {
        let source = "val a = \"/* text */\"\n/* 日本語 /* nested */ 🙂 */\nval b = \"\"\"/* raw */\"\"\"\n/* unfinished";
        let masked = super::mask(source);
        assert_eq!(source.len(), masked.len());
        assert_eq!(
            source.match_indices('\n').collect::<Vec<_>>(),
            masked.match_indices('\n').collect::<Vec<_>>()
        );
        assert!(masked.contains("\"/* text */\""));
        assert!(masked.contains("/* raw */"));
        assert!(masked.ends_with("/* unfinished"));
        assert!(!masked.contains("nested"));
    }
}
