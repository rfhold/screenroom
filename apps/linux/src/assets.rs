use std::{cell::RefCell, collections::HashMap};
#[derive(Clone)]
pub struct Pixels {
    pub data: Vec<u8>,
    pub width: i32,
    pub height: i32,
}
thread_local! {
    static ART: RefCell<crate::cache::Cache<String, Pixels>> = RefCell::new(crate::cache::Cache::new(crate::cache::PIXEL_ENTRIES, crate::cache::PIXEL_BYTES));
    static INCOMING: RefCell<HashMap<String, Pixels>> = RefCell::default();
}
pub fn install(art: HashMap<String, Pixels>) {
    merge(art);
}
pub fn pixels(url: &str) -> Option<Pixels> {
    INCOMING
        .with(|a| a.borrow().get(url).cloned())
        .or_else(|| ART.with(|a| a.borrow_mut().get(url).cloned()))
}
pub fn contains(url: &str) -> bool {
    INCOMING.with(|a| a.borrow().contains_key(url))
        || ART.with(|a| a.borrow_mut().get(url).is_some())
}
pub fn merge(art: HashMap<String, Pixels>) {
    INCOMING.with(|a| a.replace(art));
    INCOMING.with(|incoming| {
        ART.with(|cache| {
            let mut cache = cache.borrow_mut();
            for (url, pixels) in incoming.borrow().iter() {
                if cache.can_store(pixels.data.len()) {
                    cache.insert(url.clone(), pixels.clone(), pixels.data.len());
                }
            }
        });
    });
    crate::poster::refresh_pending();
    INCOMING.with(|a| a.borrow_mut().clear());
}
