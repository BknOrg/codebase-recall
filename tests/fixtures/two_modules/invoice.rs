use crate::parcel;
use crate::tax;

pub fn total() -> u32 {
    tax::apply(100)
}

pub fn send_receipt() {
    parcel::ship();
}
