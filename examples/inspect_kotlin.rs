#[path = "../src/analysis/kotlin_comments.rs"]
mod kotlin_comments;
// Print grammar errors without changing the inspected source.
fn main() -> anyhow::Result<()> {
    let path = std::env::args().nth(1).expect("Kotlin source path");
    let source = std::fs::read_to_string(path)?;
    let source = if std::env::args().any(|a| a == "--masked") {
        kotlin_comments::mask(&source)
    } else {
        source
    };
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&tree_sitter_kotlin_ng::LANGUAGE.into())?;
    let tree = parser.parse(&source, None).unwrap();
    fn walk(n: tree_sitter::Node<'_>, src: &str, count: &mut usize) {
        if n.is_error() || n.is_missing() {
            *count += 1;
            println!(
                "{}:{} {} {:?}",
                n.start_position().row + 1,
                n.start_position().column + 1,
                n.kind(),
                src.get(n.byte_range())
                    .unwrap_or("")
                    .chars()
                    .take(180)
                    .collect::<String>()
            );
        }
        let mut cursor = n.walk();
        for child in n.children(&mut cursor) {
            walk(child, src, count);
        }
    }
    let mut count = 0;
    walk(tree.root_node(), &source, &mut count);
    println!("errors={count}");
    Ok(())
}
