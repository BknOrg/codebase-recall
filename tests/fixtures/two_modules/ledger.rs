use crate::invoice;
use crate::tax;

pub fn close_books() -> u32 {
    let total = invoice::total();
    tax::apply(total)
}
