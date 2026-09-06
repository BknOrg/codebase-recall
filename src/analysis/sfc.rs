//! Single File Component (SFC) extraction for Vue (`.vue`) and Svelte (`.svelte`).
//!
//! Extracts `<script>` and `<script setup>` blocks, creates a line- and byte-offset-preserving
//! virtual TypeScript/JavaScript source, parses it with tree-sitter, and scans templates
//! for custom component references.

use crate::analysis::javascript;
use crate::analysis::{Language, ParsedFile};
use crate::cache::models::{NewRef, NewSymbol};

/// Parse a Vue or Svelte source file.
pub fn parse(source: &str, _language: Language) -> ParsedFile {
    let script_blocks = find_script_blocks(source);
    let total_lines = source.lines().count().max(1) as i64;
    let total_bytes = source.len() as i64;

    let mut out = if script_blocks.is_empty() {
        ParsedFile {
            parse_ok: true,
            ..ParsedFile::default()
        }
    } else {
        // Decide if any script block requests TypeScript. Default to TSX for SFCs
        // because TSX safely parses TS, JS, and JSX.
        let is_any_ts = script_blocks.iter().any(|b| b.is_ts);
        let parse_lang = if is_any_ts {
            Language::Tsx
        } else {
            Language::Jsx
        };

        // Create line- and byte-preserving virtual source code
        let virtual_src = build_virtual_source(source, &script_blocks);
        javascript::parse(&virtual_src, parse_lang)
    };

    // Synthesize a component default export symbol if none exists, so `import Comp from './Comp.vue'`
    // can bind to this component.
    let has_default_export = out
        .symbols
        .iter()
        .any(|s| s.name == "default" && s.is_exported);
    if !has_default_export {
        out.symbols.push(NewSymbol {
            name: "default".to_string(),
            kind: "component".to_string(),
            parent_index: None,
            is_exported: true,
            start_line: 1,
            end_line: total_lines,
            start_byte: 0,
            end_byte: total_bytes,
            signature: None,
        });
    }

    // Scan template for custom component usage (e.g. `<UserCard ... />` or `<HeaderBar>`)
    scan_template_components(source, &script_blocks, &mut out.refs);

    // Suppress parse_ok = false if there are no script blocks or minimal template
    if script_blocks.is_empty() {
        out.parse_ok = true;
    }

    out
}

#[derive(Debug, Clone, Copy)]
struct ScriptBlock {
    start_content_byte: usize,
    end_content_byte: usize,
    is_ts: bool,
}

/// Find all `<script ...> ... </script>` blocks in the source.
fn find_script_blocks(source: &str) -> Vec<ScriptBlock> {
    let mut blocks = Vec::new();
    let bytes = source.as_bytes();
    let len = bytes.len();
    let mut cursor = 0;

    while cursor < len {
        // Find next `<script` case-insensitively
        let Some(script_tag_pos) = find_tag_case_insensitive(source, cursor, "script") else {
            break;
        };

        // Ensure next char is whitespace or `>`
        let after_name = script_tag_pos + 7; // "<script".len()
        if after_name < len {
            let next_char = bytes[after_name];
            if !next_char.is_ascii_whitespace() && next_char != b'>' && next_char != b'/' {
                cursor = after_name;
                continue;
            }
        }

        // Find closing `>` of opening `<script ...>`
        let Some(open_tag_end) = find_closing_gt(source, script_tag_pos) else {
            break;
        };

        let tag_header = &source[script_tag_pos..open_tag_end];
        let is_ts = tag_header.contains("lang=\"ts\"")
            || tag_header.contains("lang='ts'")
            || tag_header.contains("lang=\"typescript\"")
            || tag_header.contains("lang='typescript'");

        let content_start = open_tag_end + 1;

        // Find closing `</script>`
        let Some(close_tag_pos) = find_tag_case_insensitive(source, content_start, "/script")
        else {
            // Unclosed script tag: take remainder of file
            blocks.push(ScriptBlock {
                start_content_byte: content_start,
                end_content_byte: len,
                is_ts,
            });
            break;
        };

        blocks.push(ScriptBlock {
            start_content_byte: content_start,
            end_content_byte: close_tag_pos,
            is_ts,
        });

        cursor = close_tag_pos + 9; // "</script>".len()
    }

    blocks
}

/// Helper to find `<tag` case-insensitively, ignoring HTML comments `<!-- ... -->`.
fn find_tag_case_insensitive(source: &str, mut start: usize, tag: &str) -> Option<usize> {
    let bytes = source.as_bytes();
    let len = bytes.len();
    let tag_bytes = tag.as_bytes();
    let tag_len = tag_bytes.len();

    while start < len {
        // Check HTML comment start
        if start + 4 <= len && &bytes[start..start + 4] == b"<!--" {
            if let Some(comment_end) = source[start + 4..].find("-->") {
                start = start + 4 + comment_end + 3;
                continue;
            } else {
                return None;
            }
        }

        if bytes[start] == b'<' && start + 1 + tag_len <= len {
            let slice = &bytes[start + 1..start + 1 + tag_len];
            if slice.eq_ignore_ascii_case(tag_bytes) {
                return Some(start);
            }
        }

        start += 1;
    }

    None
}

/// Find the `>` that closes an opening tag, skipping quoted strings in attributes.
fn find_closing_gt(source: &str, start: usize) -> Option<usize> {
    let bytes = source.as_bytes();
    let len = bytes.len();
    let mut in_single_quote = false;
    let mut in_double_quote = false;

    for i in start..len {
        let b = bytes[i];
        if b == b'"' && !in_single_quote {
            in_double_quote = !in_double_quote;
        } else if b == b'\'' && !in_double_quote {
            in_single_quote = !in_single_quote;
        } else if b == b'>' && !in_single_quote && !in_double_quote {
            return Some(i);
        }
    }
    None
}

/// Replace all non-script bytes with space (preserving `\n` and `\r`).
fn build_virtual_source(source: &str, blocks: &[ScriptBlock]) -> String {
    let mut buf = source.as_bytes().to_vec();

    let mut block_idx = 0;
    for (i, byte) in buf.iter_mut().enumerate() {
        let inside = if block_idx < blocks.len() {
            let b = &blocks[block_idx];
            if i >= b.end_content_byte {
                block_idx += 1;
                if block_idx < blocks.len() {
                    i >= blocks[block_idx].start_content_byte
                        && i < blocks[block_idx].end_content_byte
                } else {
                    false
                }
            } else {
                i >= b.start_content_byte
            }
        } else {
            false
        };

        if !inside && *byte != b'\n' && *byte != b'\r' {
            *byte = b' ';
        }
    }

    String::from_utf8(buf).unwrap_or_else(|_| source.to_string())
}

/// Scan template regions for PascalCase component tags `<ComponentName` and add them to refs.
fn scan_template_components(source: &str, script_blocks: &[ScriptBlock], refs: &mut Vec<NewRef>) {
    let bytes = source.as_bytes();
    let len = bytes.len();
    let mut i = 0;
    let mut current_line: i64 = 1;

    // Fast check: whether index i is inside a script block
    let is_in_script = |idx: usize| -> bool {
        script_blocks
            .iter()
            .any(|b| idx >= b.start_content_byte && idx < b.end_content_byte)
    };

    while i < len {
        let b = bytes[i];
        if b == b'\n' {
            current_line += 1;
            i += 1;
            continue;
        }

        // Skip HTML comments
        if i + 4 <= len && &bytes[i..i + 4] == b"<!--" {
            if let Some(end) = source[i + 4..].find("-->") {
                let skipped = &source[i..i + 4 + end + 3];
                current_line += skipped.chars().filter(|&c| c == '\n').count() as i64;
                i += 4 + end + 3;
                continue;
            }
        }

        // Skip `<style>...</style>`
        if i + 6 <= len && bytes[i] == b'<' && bytes[i + 1..i + 6].eq_ignore_ascii_case(b"style") {
            if let Some(end) = source[i..].find("</style>") {
                let skipped = &source[i..i + end + 8];
                current_line += skipped.chars().filter(|&c| c == '\n').count() as i64;
                i += end + 8;
                continue;
            }
        }

        // Check for `<` followed immediately by an uppercase ASCII character (PascalCase component)
        if b == b'<' && !is_in_script(i) && i + 1 < len && bytes[i + 1].is_ascii_uppercase() {
            let start_byte = i as i64;
            let start_line = current_line;
            let mut name_end = i + 1;
            while name_end < len
                && (bytes[name_end].is_ascii_alphanumeric() || bytes[name_end] == b'_')
            {
                name_end += 1;
            }
            let name = &source[i + 1..name_end];
            refs.push(NewRef {
                name: name.to_string(),
                ref_kind: "component".to_string(),
                receiver: None,
                start_line,
                start_byte,
            });
            i = name_end;
            continue;
        }

        i += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_vue_script_setup_and_components() {
        let vue_src = r#"<template>
  <div class="user-card">
    <HeaderBar title="Profile" />
    <!-- <OldComponent /> -->
    <UserAvatar :url="avatarUrl" />
    <h1>{{ name }}</h1>
    <button @click="sayHi">Hello</button>
  </div>
</template>

<script setup lang="ts">
import HeaderBar from './HeaderBar.vue';
import UserAvatar from './UserAvatar.vue';

const name = "Alice";
function sayHi() {
    console.log("hi");
}
</script>

<style>
.user-card { padding: 16px; }
</style>
"#;

        let parsed = parse(vue_src, Language::Vue);
        assert!(parsed.parse_ok);

        // Verify imports
        assert_eq!(parsed.imports.len(), 2);
        assert_eq!(parsed.imports[0].raw_specifier, "./HeaderBar.vue");
        assert_eq!(parsed.imports[1].raw_specifier, "./UserAvatar.vue");

        // Verify symbols
        let sym_names: Vec<&str> = parsed.symbols.iter().map(|s| s.name.as_str()).collect();
        assert!(sym_names.contains(&"sayHi"));
        assert!(sym_names.contains(&"name"));
        assert!(sym_names.contains(&"default")); // synthetic component symbol

        // Verify component template refs
        let ref_names: Vec<&str> = parsed.refs.iter().map(|r| r.name.as_str()).collect();
        assert!(ref_names.contains(&"HeaderBar"));
        assert!(ref_names.contains(&"UserAvatar"));
        assert!(!ref_names.contains(&"OldComponent")); // skipped in comment!

        // Verify line accuracy for `sayHi`
        let say_hi = parsed.symbols.iter().find(|s| s.name == "sayHi").unwrap();
        assert_eq!(say_hi.start_line, 16);
    }

    #[test]
    fn extracts_svelte_script_and_components() {
        let svelte_src = r#"<script lang="ts">
  import ItemView from './ItemView.svelte';
  export let count = 0;

  function increment() {
    count += 1;
  }
</script>

<div class="counter">
  <button on:click={increment}>Count: {count}</button>
  <ItemView {count} />
</div>
"#;

        let parsed = parse(svelte_src, Language::Svelte);
        assert!(parsed.parse_ok);

        assert_eq!(parsed.imports.len(), 1);
        assert_eq!(parsed.imports[0].raw_specifier, "./ItemView.svelte");

        let sym_names: Vec<&str> = parsed.symbols.iter().map(|s| s.name.as_str()).collect();
        assert!(sym_names.contains(&"increment"));
        assert!(sym_names.contains(&"count"));
        assert!(sym_names.contains(&"default"));

        let ref_names: Vec<&str> = parsed.refs.iter().map(|r| r.name.as_str()).collect();
        assert!(ref_names.contains(&"ItemView"));
    }
}
