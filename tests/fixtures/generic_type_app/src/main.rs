mod collector;
mod custom_buffer;

fn main() {
    eprintln!("TRACE_CALL: main::main");
    collector::collect_std_vec();
    collector::collect_custom_buffer();
}
