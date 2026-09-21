struct Settings;

impl Settings {
    fn get_string(&self, key: &str) -> String {
        String::from(key)
    }
}

fn main() {
    let settings = Settings;
    // The same spelling as `[runner] mode` in settings.toml.
    let _ = settings.get_string("runner.mode");
}
