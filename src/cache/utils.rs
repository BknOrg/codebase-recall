use crate::cache::models::NewSymbol;

/// Compute Levenshtein edit distance between two strings.
pub fn levenshtein(a: &str, b: &str) -> usize {
    let a_chars: Vec<char> = a.chars().collect();
    let b_chars: Vec<char> = b.chars().collect();
    let m = a_chars.len();
    let n = b_chars.len();
    if m == 0 {
        return n;
    }
    if n == 0 {
        return m;
    }

    let mut prev: Vec<usize> = (0..=n).collect();
    let mut curr = vec![0; n + 1];

    for i in 1..=m {
        curr[0] = i;
        for j in 1..=n {
            let cost = if a_chars[i - 1].to_ascii_lowercase() == b_chars[j - 1].to_ascii_lowercase() {
                0
            } else {
                1
            };
            curr[j] = (prev[j] + 1).min(curr[j - 1] + 1).min(prev[j - 1] + cost);
        }
        prev.clone_from_slice(&curr);
    }
    prev[n]
}

/// Innermost (narrowest) symbol whose byte range contains `byte`.
pub fn innermost_symbol(symbols: &[NewSymbol], ids: &[i64], byte: i64) -> Option<i64> {
    let mut best: Option<(i64, i64)> = None; // (width, id)
    for (i, s) in symbols.iter().enumerate() {
        if byte >= s.start_byte && byte < s.end_byte {
            let width = s.end_byte - s.start_byte;
            if best.is_none_or(|(w, _)| width < w) {
                best = Some((width, ids[i]));
            }
        }
    }
    best.map(|(_, id)| id)
}

pub fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_levenshtein_distance() {
        assert_eq!(levenshtein("kitten", "sitting"), 3);
        assert_eq!(levenshtein("build_graph", "build_grap"), 1);
        assert_eq!(levenshtein("same", "same"), 0);
        assert_eq!(levenshtein("", "test"), 4);
    }
}
