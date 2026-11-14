use std::collections::HashSet;

#[derive(Default)]
pub struct Generation(std::cell::Cell<u64>);
impl Generation {
    pub fn get(&self) -> u64 {
        self.0.get()
    }
    pub fn advance(&self) -> u64 {
        self.0.set(self.0.get().wrapping_add(1));
        self.get()
    }
    pub fn is_current(&self, request: u64) -> bool {
        self.get() == request
    }
}

#[derive(Default)]
pub struct Paging {
    pub ids: Vec<String>,
    pub next: String,
    pub busy: bool,
    pub failed: bool,
    pub stopped: bool,
    tokens: HashSet<String>,
    pages: usize,
    received: usize,
}
impl Paging {
    pub fn begin(&mut self) -> Option<String> {
        if self.busy || self.stopped || self.next.is_empty() {
            return None;
        }
        self.busy = true;
        self.failed = false;
        Some(self.next.clone())
    }
    pub fn accept(
        &mut self,
        ids: impl IntoIterator<Item = String>,
        next: String,
    ) -> Result<Vec<String>, ()> {
        let incoming = ids.into_iter().take(10_001).collect::<Vec<_>>();
        if incoming.len() > 10_000 - self.received {
            self.fail();
            return Err(());
        }
        self.busy = false;
        self.failed = false;
        self.pages += 1;
        self.received += incoming.len();
        let mut known = self.ids.iter().cloned().collect::<HashSet<_>>();
        let added = incoming
            .into_iter()
            .filter(|id| known.insert(id.clone()))
            .collect::<Vec<_>>();
        self.ids.extend(added.iter().cloned());
        self.stopped = self.pages >= 512 || (!next.is_empty() && !self.tokens.insert(next.clone()));
        self.next = if self.stopped { String::new() } else { next };
        Ok(added)
    }
    pub fn fail(&mut self) {
        self.busy = false;
        self.failed = true;
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn context_change_rejects_pending_old_pages() {
        let generation = Generation::default();
        let old = generation.advance();
        let current = generation.advance();
        let mut pages = Paging::default();
        if generation.is_current(old) {
            pages.accept(["stale".into()], "old".into()).unwrap();
        }
        assert!(pages.ids.is_empty());
        if generation.is_current(current) {
            pages.accept(["current".into()], "next".into()).unwrap();
        }
        assert_eq!(pages.ids, ["current"]);
        assert_eq!(pages.next, "next");
    }
    #[test]
    fn first_page_append_singleflight_and_retry() {
        let mut p = Paging::default();
        p.accept((0..100).map(|i| i.to_string()), "next".into())
            .unwrap();
        assert_eq!(p.ids.len(), 100);
        assert_eq!(p.begin().as_deref(), Some("next"));
        assert!(p.begin().is_none());
        p.fail();
        assert_eq!(p.ids.len(), 100);
        assert_eq!(p.begin().as_deref(), Some("next"));
        assert_eq!(
            p.accept(["99".into(), "101".into()], "last".into())
                .unwrap(),
            ["101"]
        );
        assert_eq!(p.ids.last().unwrap(), "101");
        p.accept([], "next".into()).unwrap();
        assert!(p.stopped);
        assert!(p.begin().is_none());
    }
    #[test]
    fn terminal_overflow_rejects_before_state_changes_and_retry_retains_token() {
        let mut p = Paging::default();
        p.accept((0..9950).map(|i| i.to_string()), "retained".into())
            .unwrap();
        let ids = p.ids.clone();
        let tokens = p.tokens.clone();
        assert_eq!(p.begin().as_deref(), Some("retained"));
        assert!(
            p.accept((9950..10050).map(|i| i.to_string()), String::new())
                .is_err()
        );
        assert_eq!(p.ids, ids);
        assert_eq!(p.next, "retained");
        assert_eq!(p.tokens, tokens);
        assert_eq!((p.received, p.pages), (9950, 1));
        assert!(p.failed);
        assert_eq!(p.begin().as_deref(), Some("retained"));
        p.accept((9950..10000).map(|i| i.to_string()), String::new())
            .unwrap();
        assert_eq!(p.ids.len(), 10000);
        assert_eq!(p.received, 10000);
        assert!(!p.failed);
        assert!(p.next.is_empty());
    }
    #[test]
    fn duplicate_heavy_pages_consume_the_received_record_budget() {
        let mut p = Paging::default();
        for page in 1..=100 {
            p.accept((0..100).map(|_| "same".into()), page.to_string())
                .unwrap();
        }
        assert_eq!((p.ids.len(), p.received), (1, 10000));
        let tokens = p.tokens.clone();
        assert!(p.accept(["same".into()], String::new()).is_err());
        assert_eq!((p.ids.len(), p.received, p.pages), (1, 10000, 100));
        assert_eq!(p.next, "100");
        assert_eq!(p.tokens, tokens);
        assert_eq!(p.begin().as_deref(), Some("100"));
        p.accept([], String::new()).unwrap();
        assert!(p.next.is_empty());
        assert_eq!(p.ids, ["same"]);
    }
}
