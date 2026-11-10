use std::{cell::RefCell, collections::HashMap};
#[derive(Clone)]
pub struct Pixels {
    pub data: Vec<u8>,
    pub width: i32,
    pub height: i32,
}
thread_local! { static ART: RefCell<HashMap<String, Pixels>> = RefCell::default(); }
pub fn install(art: HashMap<String, Pixels>) {
    crate::poster::clear_cache();
    ART.with(|a| a.replace(art));
}
pub fn pixels(url: &str) -> Option<Pixels> {
    ART.with(|a| a.borrow().get(url).cloned())
}
pub fn merge(art: HashMap<String, Pixels>) {
    ART.with(|a| a.borrow_mut().extend(art));
}
