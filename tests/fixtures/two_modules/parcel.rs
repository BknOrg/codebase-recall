use crate::label;
use crate::route;

pub fn ship() {
    route::plan();
    label::print();
}
