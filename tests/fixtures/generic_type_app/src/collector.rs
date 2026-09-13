use crate::custom_buffer::CustomBuffer;

pub fn collect_std_vec() {
    eprintln!("TRACE_CALL: collector::collect_std_vec");
    let mut v: Vec<String> = Vec::new();
    v.push(String::from("std_alpha"));
}

pub fn collect_custom_buffer() {
    eprintln!("TRACE_CALL: collector::collect_custom_buffer");
    let mut buf: CustomBuffer<String> = CustomBuffer::new();
    buf.push(String::from("custom_alpha"));
}
