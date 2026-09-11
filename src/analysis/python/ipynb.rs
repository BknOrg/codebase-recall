// src/dump/ipynb.rs
use crate::dump::formatter::calculate_fence;
use serde::Deserialize;

#[derive(Deserialize)]
struct Notebook {
    cells: Vec<Cell>,
}

#[derive(Deserialize)]
struct Cell {
    cell_type: String, // "code" | "markdown" | "raw"
    source: CellSource,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum CellSource {
    Lines(Vec<String>),
    Single(String),
}

impl CellSource {
    fn to_string(&self) -> String {
        match self {
            CellSource::Lines(lines) => lines.join(""),
            CellSource::Single(s) => s.clone(),
        }
    }
}

/// Mengubah JSON .ipynb menjadi Markdown bersih:
/// - Sel markdown langsung dicetak sebagai teks biasa.
/// - Sel code dibungkus ke dalam ```python ... ```.
/// - Seluruh `outputs` (gambar base64, dsb.) dan metadata dibuang 100%.
pub fn cleaning_ipynb(raw_json: &str) -> Option<String> {
    let notebook: Notebook = serde_json::from_str(raw_json).ok()?;
    let mut out = String::new();

    for (i, cell) in notebook.cells.iter().enumerate() {
        let text = cell.source.to_string();
        let trimmed = text.trim();
        if trimmed.is_empty() {
            continue;
        }

        match cell.cell_type.as_str() {
            "markdown" => {
                out.push_str(trimmed);
                out.push_str("\n\n");
            }
            "code" => {
                let fence = calculate_fence(trimmed);
                out.push_str(&format!("<!-- Notebook Cell [{}] -->\n", i + 1));
                out.push_str(&format!("{fence}python\n"));
                out.push_str(trimmed);
                out.push_str(&format!("\n{}\n\n", fence));
            }
            _ => {}
        }
    }

    Some(out)
}
