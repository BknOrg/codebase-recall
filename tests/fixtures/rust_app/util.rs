pub fn greet(name: &str) -> String {
    decorate(name)
}

fn decorate(name: &str) -> String {
    format!("hello, {name}")
}
