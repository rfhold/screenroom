//! Bounded metadata observations, not a replay log or removal evidence.
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

/// Maximum encoded durable checkpoint size, including the bounded seen-ID set.
pub const MAX_CHECKPOINT_BYTES: usize = 8 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FallbackReason {
    InvalidVersion,
    UnsupportedVersion,
    FilterRejected,
    UnsupportedLibrary,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Capability {
    TimestampLowerBound,
    FullScanRequired(FallbackReason),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValidationError {
    Timestamp,
    Window,
    Scope,
    Bounds,
    Duplicate,
    Exhausted,
    Encoding,
    Version,
    Binding,
    State,
}

/// Strict UTC RFC3339, with up to seven fractional digits (provider tick precision).
/// DateCreated must never be used as a substitute for a modification watermark.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct Timestamp(String);

impl Timestamp {
    pub fn parse(value: &str) -> Result<Self, ValidationError> {
        let invalid = || ValidationError::Timestamp;
        let b = value.as_bytes();
        if !(20..=28).contains(&b.len())
            || b[4] != b'-'
            || b[7] != b'-'
            || b[10] != b'T'
            || b[13] != b':'
            || b[16] != b':'
            || b[b.len() - 1] != b'Z'
        {
            return Err(invalid());
        }
        let number = |range: std::ops::Range<usize>| -> Result<u32, ValidationError> {
            let bytes = &b[range];
            if !bytes.iter().all(u8::is_ascii_digit) {
                return Err(invalid());
            }
            Ok(bytes.iter().fold(0, |v, n| v * 10 + u32::from(n - b'0')))
        };
        let year = number(0..4)?;
        let month = number(5..7)?;
        let day = number(8..10)?;
        let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
        let days = match month {
            1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
            4 | 6 | 9 | 11 => 30,
            2 if leap => 29,
            2 => 28,
            _ => 0,
        };
        if year == 0
            || day == 0
            || day > days
            || number(11..13)? > 23
            || number(14..16)? > 59
            || number(17..19)? > 59
        {
            return Err(invalid());
        }
        let fraction = if b.len() == 20 {
            ""
        } else {
            if b[19] != b'.' || b.len() < 22 || !b[20..b.len() - 1].iter().all(u8::is_ascii_digit) {
                return Err(invalid());
            }
            std::str::from_utf8(&b[20..b.len() - 1]).map_err(|_| invalid())?
        };
        let prefix = std::str::from_utf8(&b[..19]).map_err(|_| invalid())?;
        Ok(Self(format!("{prefix}.{fraction:0<7}Z")))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Caller supplies an overlapping lower bound, normally previous watermark minus
/// configured overlap. run_start is the next candidate watermark, not an upper
/// filter: later modifications may appear. Clock skew/rollback beyond overlap and
/// moving offset pages require periodic full reconciliation. Commit run_start only
/// after successful exhaustion and atomic persistence of all observations.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Window {
    lower_bound: Timestamp,
    run_start: Timestamp,
}
impl Window {
    pub fn new(lower_bound: Timestamp, run_start: Timestamp) -> Result<Self, ValidationError> {
        if lower_bound > run_start {
            return Err(ValidationError::Window);
        }
        Ok(Self {
            lower_bound,
            run_start,
        })
    }
    pub fn lower_bound(&self) -> &Timestamp {
        &self.lower_bound
    }
    pub fn run_start(&self) -> &Timestamp {
        &self.run_start
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Scope {
    source_key: String,
    library_id: String,
}
impl Scope {
    pub fn new(source_key: &str, library_id: &str) -> Result<Self, ValidationError> {
        if !valid_identifier(source_key) || !valid_identifier(library_id) {
            return Err(ValidationError::Scope);
        }
        Ok(Self {
            source_key: source_key.into(),
            library_id: library_id.into(),
        })
    }
    pub fn source_key(&self) -> &str {
        &self.source_key
    }
    pub fn library_id(&self) -> &str {
        &self.library_id
    }
}
fn valid_identifier(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 1024
        && id != "."
        && id != ".."
        && !id.chars().any(char::is_control)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Query {
    scope: Scope,
    window: Window,
    page_size: u32,
    max_items: u32,
    max_pages: u32,
}
impl Query {
    pub fn new(
        scope: Scope,
        window: Window,
        page_size: u32,
        max_items: u32,
        max_pages: u32,
    ) -> Result<Self, ValidationError> {
        if !(1..=100).contains(&page_size)
            || !(1..=50_000).contains(&max_items)
            || !(1..=50_000).contains(&max_pages)
            || page_size > max_items
        {
            return Err(ValidationError::Bounds);
        }
        Ok(Self {
            scope,
            window,
            page_size,
            max_items,
            max_pages,
        })
    }
    pub fn scope(&self) -> &Scope {
        &self.scope
    }
    pub fn window(&self) -> &Window {
        &self.window
    }
    pub fn page_size(&self) -> u32 {
        self.page_size
    }
}

/// Traversal guard. Persist the encoded checkpoint atomically with page observations.
/// Restoring validates structure, not authenticity: storage must remain trusted.
#[derive(Debug, Clone)]
pub struct Checkpoint {
    query: Query,
    next_offset: Option<u32>,
    pages: u32,
    seen: BTreeSet<String>,
    reported_total: Option<u32>,
}
impl Checkpoint {
    pub fn new(query: Query) -> Self {
        Self {
            query,
            next_offset: Some(0),
            pages: 0,
            seen: BTreeSet::new(),
            reported_total: None,
        }
    }
    pub fn query(&self) -> &Query {
        &self.query
    }
    pub fn next_offset(&self) -> Option<u32> {
        self.next_offset
    }
    pub fn is_complete(&self) -> bool {
        self.next_offset.is_none()
    }
    pub fn observed_count(&self) -> usize {
        self.seen.len()
    }
    /// Versioned JSON, capped before allocation can exceed MAX_CHECKPOINT_BYTES.
    pub fn serialize(&self) -> Result<Vec<u8>, ValidationError> {
        #[derive(Serialize)]
        struct Encoded<'a> {
            version: u32,
            query: &'a Query,
            next_offset: Option<u32>,
            pages: u32,
            seen: &'a BTreeSet<String>,
            reported_total: Option<u32>,
        }
        struct Bounded(Vec<u8>);
        impl std::io::Write for Bounded {
            fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
                if bytes.len() > MAX_CHECKPOINT_BYTES - self.0.len() {
                    return Err(std::io::Error::other("checkpoint size limit"));
                }
                self.0.extend_from_slice(bytes);
                Ok(bytes.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let mut bytes = Bounded(Vec::new());
        serde_json::to_writer(
            &mut bytes,
            &Encoded {
                version: 1,
                query: &self.query,
                next_offset: self.next_offset,
                pages: self.pages,
                seen: &self.seen,
                reported_total: self.reported_total,
            },
        )
        .map_err(|_| ValidationError::Bounds)?;
        Ok(bytes.0)
    }

    /// Restore only against the run's independently loaded immutable query. Changed
    /// source/library, window, page size or budgets cannot reuse an old cursor.
    /// This is not a MAC: a coherently forged checkpoint cannot be detected here.
    pub fn restore(expected_query: &Query, bytes: &[u8]) -> Result<Self, ValidationError> {
        if bytes.len() > MAX_CHECKPOINT_BYTES {
            return Err(ValidationError::Bounds);
        }
        let wire: CheckpointWire =
            serde_json::from_slice(bytes).map_err(|_| ValidationError::Encoding)?;
        if wire.version != 1 {
            return Err(ValidationError::Version);
        }
        let query = Query::new(
            Scope::new(&wire.query.scope.source_key, &wire.query.scope.library_id)?,
            Window::new(
                Timestamp::parse(&wire.query.window.lower_bound)?,
                Timestamp::parse(&wire.query.window.run_start)?,
            )?,
            wire.query.page_size,
            wire.query.max_items,
            wire.query.max_pages,
        )?;
        if &query != expected_query {
            return Err(ValidationError::Binding);
        }
        if wire.seen.len() > query.max_items as usize
            || wire.pages > query.max_pages
            || wire.seen.iter().any(|id| !valid_identifier(id))
        {
            return Err(ValidationError::State);
        }
        let count = wire.seen.len() as u32;
        let seen: BTreeSet<_> = wire.seen.into_iter().collect();
        if seen.len() != count as usize {
            return Err(ValidationError::Duplicate);
        }
        let valid = if wire.pages == 0 {
            count == 0 && wire.next_offset == Some(0) && wire.reported_total.is_none()
        } else if let Some(total) = wire.reported_total {
            // Each nonempty page contributes 1..=page_size unique IDs. Only the
            // first, exhausted page may be empty. Short pages are legal.
            let page_counts = if count == 0 {
                wire.pages == 1 && total == 0
            } else {
                wire.pages <= count
                    && u64::from(count) <= u64::from(wire.pages) * u64::from(query.page_size)
            };
            page_counts
                && count <= total
                && total <= query.max_items
                && match wire.next_offset {
                    Some(offset) => offset == count && count > 0 && count < total,
                    None => count == total,
                }
        } else {
            false
        };
        if !valid {
            return Err(ValidationError::State);
        }
        Ok(Self {
            query,
            next_offset: wire.next_offset,
            pages: wire.pages,
            seen,
            reported_total: wire.reported_total,
        })
    }
    pub(crate) fn request_offset(&self) -> Result<u32, ValidationError> {
        let offset = self.next_offset.ok_or(ValidationError::Exhausted)?;
        if self.pages >= self.query.max_pages || self.seen.len() >= self.query.max_items as usize {
            return Err(ValidationError::Bounds);
        }
        Ok(offset)
    }
    pub(crate) fn accept(
        &mut self,
        ids: &[String],
        next: Option<u32>,
        total: u32,
    ) -> Result<(), ValidationError> {
        let offset = self.request_offset()?;
        let unique: BTreeSet<_> = ids.iter().cloned().collect();
        if unique.len() != ids.len()
            || unique
                .iter()
                .any(|id| !valid_identifier(id) || self.seen.contains(id))
        {
            return Err(ValidationError::Duplicate);
        }
        let count = offset
            .checked_add(ids.len() as u32)
            .ok_or(ValidationError::Bounds)?;
        if ids.len() > self.query.page_size as usize
            || count > self.query.max_items
            || total > self.query.max_items
            || self.reported_total.is_some_and(|old| old != total)
            || count > total
            || next.is_some_and(|n| n != count || n <= offset || count >= total)
            || (next.is_none() && count != total)
        {
            return Err(ValidationError::Bounds);
        }
        self.seen.extend(unique);
        self.pages += 1;
        self.next_offset = next;
        self.reported_total = Some(total);
        Ok(())
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CheckpointWire {
    version: u32,
    query: QueryWire,
    #[serde(deserialize_with = "required_option")]
    next_offset: Option<u32>,
    pages: u32,
    #[serde(deserialize_with = "bounded_seen")]
    seen: Vec<String>,
    #[serde(deserialize_with = "required_option")]
    reported_total: Option<u32>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct QueryWire {
    scope: ScopeWire,
    window: WindowWire,
    page_size: u32,
    max_items: u32,
    max_pages: u32,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ScopeWire {
    source_key: String,
    library_id: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WindowWire {
    lower_bound: String,
    run_start: String,
}
fn required_option<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<u32>, D::Error> {
    Option::<u32>::deserialize(deserializer)
}
fn bounded_seen<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Vec<String>, D::Error> {
    struct Seen;
    impl<'de> serde::de::Visitor<'de> for Seen {
        type Value = Vec<String>;
        fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("at most 50000 valid identifiers")
        }
        fn visit_seq<A: serde::de::SeqAccess<'de>>(
            self,
            mut seq: A,
        ) -> Result<Self::Value, A::Error> {
            let mut ids = Vec::new();
            while let Some(id) = seq.next_element::<String>()? {
                if ids.len() >= 50_000 || !valid_identifier(&id) {
                    return Err(serde::de::Error::custom("invalid seen identifiers"));
                }
                ids.push(id);
            }
            Ok(ids)
        }
    }
    deserializer.deserialize_seq(Seen)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn query() -> Query {
        Query::new(
            Scope::new("source", "library").unwrap(),
            Window::new(
                Timestamp::parse("2026-11-12T10:00:00Z").unwrap(),
                Timestamp::parse("2026-11-12T10:05:00Z").unwrap(),
            )
            .unwrap(),
            1,
            2,
            2,
        )
        .unwrap()
    }

    #[test]
    fn timestamps_validate_calendar_and_normalize_precision() {
        assert_eq!(
            Timestamp::parse("2024-02-29T01:02:03.1Z").unwrap().as_str(),
            "2024-02-29T01:02:03.1000000Z"
        );
        for value in [
            "2026-02-29T01:02:03Z",
            "0000-01-01T01:02:03Z",
            "2024-13-01T01:02:03Z",
            "2024-01-01T24:02:03Z",
            "2024-01-01T01:60:03Z",
            "2024-01-01T01:02:60Z",
            "2024-01-01T01:02:03.12345678Z",
            "2024-01-01T01:02:03+00:00",
            "é024-01-01T01:02:03Z",
        ] {
            assert!(Timestamp::parse(value).is_err(), "{value}");
        }
        let q = query();
        assert!(Window::new(q.window.run_start.clone(), q.window.lower_bound.clone()).is_err());
    }

    #[test]
    fn checkpoint_rejections_are_atomic() {
        let mut checkpoint = Checkpoint::new(query());
        checkpoint.accept(&["a".into()], Some(1), 2).unwrap();
        assert_eq!(
            checkpoint.accept(&["a".into()], None, 2),
            Err(ValidationError::Duplicate)
        );
        assert_eq!(
            checkpoint.accept(&["b".into()], None, 3),
            Err(ValidationError::Bounds)
        );
        assert_eq!(checkpoint.next_offset(), Some(1));
        assert_eq!(checkpoint.observed_count(), 1);
        checkpoint.accept(&["b".into()], None, 2).unwrap();
        assert!(checkpoint.is_complete());
        assert_eq!(checkpoint.request_offset(), Err(ValidationError::Exhausted));
    }

    #[test]
    fn query_caps_empty_pages_and_capacity_are_explicit() {
        let mut q = query();
        q.max_pages = 1;
        let mut checkpoint = Checkpoint::new(q);
        checkpoint.accept(&["a".into()], Some(1), 2).unwrap();
        assert_eq!(checkpoint.request_offset(), Err(ValidationError::Bounds));
        assert!(!checkpoint.is_complete());
        let mut empty = Checkpoint::new(query());
        empty.accept(&[], None, 0).unwrap();
        assert!(empty.is_complete());
        let mut capped = Checkpoint::new(query());
        assert_eq!(
            capped.accept(&["a".into()], Some(1), 3),
            Err(ValidationError::Bounds)
        );
        assert_eq!(capped.observed_count(), 0);
    }

    #[test]
    fn durable_checkpoint_roundtrip_preserves_guards_and_exact_exhaustion() {
        let q = query();
        let initial = Checkpoint::new(q.clone());
        let mut restored = Checkpoint::restore(&q, &initial.serialize().unwrap()).unwrap();
        assert_eq!(restored.next_offset(), Some(0));
        restored.accept(&["a".into()], Some(1), 2).unwrap();
        let encoded = restored.serialize().unwrap();
        let mut interrupted = Checkpoint::restore(&q, &encoded).unwrap();
        assert_eq!(interrupted.serialize().unwrap(), encoded);
        assert_eq!(interrupted.next_offset(), Some(1));
        assert_eq!(
            interrupted.accept(&["a".into()], None, 2),
            Err(ValidationError::Duplicate)
        );
        assert_eq!(
            interrupted.accept(&["b".into()], None, 3),
            Err(ValidationError::Bounds)
        );
        interrupted.accept(&["b".into()], None, 2).unwrap();
        let complete = Checkpoint::restore(&q, &interrupted.serialize().unwrap()).unwrap();
        assert!(complete.is_complete());
        assert_eq!(complete.observed_count(), 2);
        assert_eq!(complete.request_offset(), Err(ValidationError::Exhausted));
        let mut empty = Checkpoint::new(q.clone());
        empty.accept(&[], None, 0).unwrap();
        assert!(
            Checkpoint::restore(&q, &empty.serialize().unwrap())
                .unwrap()
                .is_complete()
        );
    }

    #[test]
    fn restore_rejects_malformed_and_inconsistent_state() {
        let q = query();
        let mut checkpoint = Checkpoint::new(q.clone());
        checkpoint.accept(&["a".into()], Some(1), 2).unwrap();
        let encoded = checkpoint.serialize().unwrap();
        let original: serde_json::Value = serde_json::from_slice(&encoded).unwrap();
        for (pointer, value) in [
            ("/version", serde_json::json!(2)),
            ("/version", serde_json::json!("1")),
            ("/next_offset", serde_json::json!(0)),
            ("/next_offset", serde_json::json!(2)),
            ("/next_offset", serde_json::Value::Null),
            ("/next_offset", serde_json::json!(-1)),
            ("/pages", serde_json::json!(0)),
            ("/pages", serde_json::json!(2)),
            ("/pages", serde_json::json!(3)),
            ("/seen", serde_json::json!([])),
            ("/seen", serde_json::json!(["a", "a"])),
            ("/seen", serde_json::json!(["a", "b", "c"])),
            ("/seen", serde_json::json!([".."])),
            ("/seen", serde_json::json!(["bad\n"])),
            ("/reported_total", serde_json::Value::Null),
            ("/reported_total", serde_json::json!(0)),
            ("/reported_total", serde_json::json!(1)),
            ("/reported_total", serde_json::json!(3)),
            ("/query/page_size", serde_json::json!(0)),
            ("/query/page_size", serde_json::json!(101)),
            ("/query/max_items", serde_json::json!(50001)),
            ("/query/max_pages", serde_json::json!(0)),
            (
                "/query/window/lower_bound",
                serde_json::json!("2026-02-29T00:00:00Z"),
            ),
            (
                "/query/window/lower_bound",
                serde_json::json!("2027-01-01T00:00:00Z"),
            ),
            ("/query/scope/library_id", serde_json::json!("..")),
        ] {
            let mut invalid = original.clone();
            *invalid.pointer_mut(pointer).unwrap() = value;
            assert!(
                Checkpoint::restore(&q, &serde_json::to_vec(&invalid).unwrap()).is_err(),
                "{pointer}: {invalid}"
            );
        }
        for field in [
            "version",
            "query",
            "next_offset",
            "pages",
            "seen",
            "reported_total",
        ] {
            let mut invalid = original.clone();
            invalid.as_object_mut().unwrap().remove(field);
            assert!(
                Checkpoint::restore(&q, &serde_json::to_vec(&invalid).unwrap()).is_err(),
                "missing {field}"
            );
        }
        for pointer in ["", "/query", "/query/scope", "/query/window"] {
            let mut invalid = original.clone();
            invalid
                .pointer_mut(pointer)
                .unwrap()
                .as_object_mut()
                .unwrap()
                .insert("unexpected".into(), true.into());
            assert_eq!(
                Checkpoint::restore(&q, &serde_json::to_vec(&invalid).unwrap()).unwrap_err(),
                ValidationError::Encoding
            );
        }
        let duplicate = String::from_utf8(encoded)
            .unwrap()
            .replace("\"version\":1", "\"version\":1,\"version\":1");
        assert_eq!(
            Checkpoint::restore(&q, duplicate.as_bytes()).unwrap_err(),
            ValidationError::Encoding
        );
        for bytes in [b"".as_slice(), b"{", b"null", b"[]", b"{} trailing"] {
            assert!(Checkpoint::restore(&q, bytes).is_err());
        }
    }

    #[test]
    fn restore_is_bound_to_every_immutable_query_field() {
        let q = query();
        let bytes = Checkpoint::new(q.clone()).serialize().unwrap();
        let mut variants = Vec::new();
        let mut changed = q.clone();
        changed.scope.source_key = "other-provider".into();
        variants.push(changed);
        let mut changed = q.clone();
        changed.scope.library_id = "other-library".into();
        variants.push(changed);
        let mut changed = q.clone();
        changed.window.lower_bound = Timestamp::parse("2026-11-12T09:59:59Z").unwrap();
        variants.push(changed);
        let mut changed = q.clone();
        changed.window.run_start = Timestamp::parse("2026-11-12T10:05:01Z").unwrap();
        variants.push(changed);
        let mut changed = q.clone();
        changed.page_size = 2;
        variants.push(changed);
        let mut changed = q.clone();
        changed.max_items = 3;
        variants.push(changed);
        let mut changed = q.clone();
        changed.max_pages = 3;
        variants.push(changed);
        for expected in variants {
            assert_eq!(
                Checkpoint::restore(&expected, &bytes).unwrap_err(),
                ValidationError::Binding
            );
            let changed = Checkpoint::new(expected).serialize().unwrap();
            assert_eq!(
                Checkpoint::restore(&q, &changed).unwrap_err(),
                ValidationError::Binding
            );
        }
    }

    #[test]
    fn durable_checkpoint_limits_and_exhausted_budget_remain_explicit() {
        let mut q = query();
        q.max_pages = 1;
        let mut checkpoint = Checkpoint::new(q.clone());
        checkpoint.accept(&["a".into()], Some(1), 2).unwrap();
        let restored = Checkpoint::restore(&q, &checkpoint.serialize().unwrap()).unwrap();
        assert!(!restored.is_complete());
        assert_eq!(restored.request_offset(), Err(ValidationError::Bounds));
        assert_eq!(
            Checkpoint::restore(&q, &vec![b' '; MAX_CHECKPOINT_BYTES + 1]).unwrap_err(),
            ValidationError::Bounds
        );
        let mut oversized = Checkpoint::new(query());
        oversized.seen = (0..10000)
            .map(|n| format!("{n:05}{}", "x".repeat(900)))
            .collect();
        assert_eq!(oversized.serialize(), Err(ValidationError::Bounds));
        let mut wire: serde_json::Value =
            serde_json::from_slice(&Checkpoint::new(query()).serialize().unwrap()).unwrap();
        wire["seen"] = serde_json::json!(vec!["a"; 50001]);
        assert_eq!(
            Checkpoint::restore(&query(), &serde_json::to_vec(&wire).unwrap()).unwrap_err(),
            ValidationError::Encoding
        );
    }
}
