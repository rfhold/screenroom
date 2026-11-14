use std::{borrow::Borrow, collections::HashMap, hash::Hash};

pub const PIXEL_ENTRIES: usize = 256;
pub const PIXEL_BYTES: usize = 96 * 1024 * 1024;
pub const TEXTURE_ENTRIES: usize = 512;
pub const TEXTURE_BYTES: usize = 32 * 1024 * 1024;

pub struct LiveIndex<K, W>(HashMap<K, W>);
impl<K, W> Default for LiveIndex<K, W> {
    fn default() -> Self {
        Self(HashMap::new())
    }
}
impl<K: Eq + Hash, W> LiveIndex<K, W> {
    pub fn insert(&mut self, key: K, weak: W) {
        self.0.insert(key, weak);
    }
    pub fn get<T>(&self, key: &K, upgrade: impl FnOnce(&W) -> Option<T>) -> Option<T> {
        self.0.get(key).and_then(upgrade)
    }
    pub fn prune(&mut self, mut alive: impl FnMut(&W) -> bool) {
        self.0.retain(|_, weak| alive(weak));
    }
}

struct Entry<V> {
    value: V,
    bytes: usize,
    used: u64,
}

pub struct Cache<K, V> {
    entries: HashMap<K, Entry<V>>,
    bytes: usize,
    max_entries: usize,
    max_bytes: usize,
    clock: u64,
}
impl<K: Eq + Hash + Clone, V> Cache<K, V> {
    pub fn new(max_entries: usize, max_bytes: usize) -> Self {
        Self {
            entries: HashMap::new(),
            bytes: 0,
            max_entries,
            max_bytes,
            clock: 0,
        }
    }
    pub fn get<Q: Eq + Hash + ?Sized>(&mut self, key: &Q) -> Option<&V>
    where
        K: Borrow<Q>,
    {
        self.clock = self.clock.wrapping_add(1);
        let entry = self.entries.get_mut(key)?;
        entry.used = self.clock;
        Some(&entry.value)
    }
    pub fn insert(&mut self, key: K, value: V, bytes: usize) -> bool {
        if !self.can_store(bytes) {
            return false;
        }
        if let Some(old) = self.entries.remove(&key) {
            self.bytes -= old.bytes;
        }
        while self.entries.len() >= self.max_entries
            || self.bytes.saturating_add(bytes) > self.max_bytes
        {
            let Some(oldest) = self
                .entries
                .iter()
                .min_by_key(|(_, entry)| entry.used)
                .map(|(key, _)| key.clone())
            else {
                break;
            };
            self.bytes -= self.entries.remove(&oldest).unwrap().bytes;
        }
        self.clock = self.clock.wrapping_add(1);
        self.bytes += bytes;
        self.entries.insert(
            key,
            Entry {
                value,
                bytes,
                used: self.clock,
            },
        );
        true
    }
    pub fn can_store(&self, bytes: usize) -> bool {
        self.max_entries > 0 && bytes <= self.max_bytes
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::rc::Rc;
    #[test]
    fn weak_index_reuses_live_images_after_eviction_and_prunes_departed_contexts() {
        let mut cache = Cache::new(1, 4);
        let mut live_index = LiveIndex::default();
        let widget = Rc::new("texture");
        cache.insert("home", widget.clone(), 4);
        live_index.insert("home", Rc::downgrade(&widget));
        cache.insert("append", Rc::new("other"), 4);
        assert!(cache.get(&"home").is_none());
        let watch_update = live_index.get(&"home", std::rc::Weak::upgrade).unwrap();
        assert!(Rc::ptr_eq(&widget, &watch_update));
        drop(widget);
        drop(watch_update);
        live_index.prune(|weak| weak.upgrade().is_some());
        assert!(live_index.0.is_empty());
    }
    #[test]
    fn production_cache_limits_bound_repeated_contexts() {
        for (entries, bytes) in [
            (PIXEL_ENTRIES, PIXEL_BYTES),
            (TEXTURE_ENTRIES, TEXTURE_BYTES),
        ] {
            let mut cache = Cache::new(entries, bytes);
            for context in 0..1000 {
                cache.insert(context, context, bytes / 3);
                assert!(cache.bytes <= bytes);
                assert!(cache.entries.len() <= entries);
            }
            assert!(cache.get(&0).is_none());
            assert_eq!(cache.get(&999), Some(&999));
        }
    }
    #[test]
    fn byte_budget_evicts_least_recently_used_entry() {
        let mut cache = Cache::new(10, 10);
        cache.insert("old", 1, 4);
        cache.insert("keep", 2, 4);
        assert_eq!(cache.get(&"old"), Some(&1));
        cache.insert("new", 3, 4);
        assert!(cache.get(&"keep").is_none());
        assert_eq!(cache.bytes, 8);
        assert_eq!(cache.entries.len(), 2);
    }
    #[test]
    fn entry_budget_applies_even_to_zero_byte_records() {
        let mut cache = Cache::new(2, 100);
        for i in 0..100 {
            cache.insert(i, i, 0);
        }
        assert_eq!(cache.entries.len(), 2);
        assert_eq!(cache.get(&99), Some(&99));
        assert!(cache.get(&0).is_none());
    }
    #[test]
    fn context_churn_and_eviction_preserve_live_widget_ownership() {
        let mut cache = Cache::new(2, 8);
        let live = Rc::new("stable texture");
        cache.insert("home", live.clone(), 4);
        cache.insert("library", Rc::new("library texture"), 4);
        let watched_update = cache.get(&"home").unwrap().clone();
        assert!(Rc::ptr_eq(&live, &watched_update));
        cache.insert("append", Rc::new("append texture"), 4);
        assert!(Rc::ptr_eq(&live, cache.get(&"home").unwrap()));
        cache.insert("next-context", Rc::new("next"), 4);
        cache.insert("another-context", Rc::new("another"), 4);
        assert!(cache.get(&"home").is_none());
        assert_eq!(*live, "stable texture");
        assert_eq!(*watched_update, "stable texture");
        assert!(cache.bytes <= 8);
    }
    #[test]
    fn replacements_account_bytes_and_oversized_entries_do_not_flush_cache() {
        let mut cache = Cache::new(2, 8);
        cache.insert("stable", 1, 4);
        cache.insert("stable", 2, 6);
        assert_eq!(cache.bytes, 6);
        assert!(!cache.insert("oversized", 3, 9));
        assert_eq!(cache.get(&"stable"), Some(&2));
        assert_eq!(cache.bytes, 6);
    }
}
