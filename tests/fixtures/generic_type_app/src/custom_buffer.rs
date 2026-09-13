pub struct CustomBuffer<T> {
    items: Vec<T>,
}

impl<T> CustomBuffer<T> {
    pub fn new() -> Self {
        eprintln!("TRACE_CALL: custom_buffer::CustomBuffer::new");
        Self { items: Vec::new() }
    }

    pub fn push(&mut self, item: T) {
        eprintln!("TRACE_CALL: custom_buffer::CustomBuffer::push");
        self.items.push(item);
    }
}
