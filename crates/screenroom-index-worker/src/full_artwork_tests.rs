use super::{ArtworkStep, Cursor};
use crate::live;
use serde_json::{Value, json};

fn inputs() -> Vec<Value> {
    ["Logo", "Primary", "Thumb", "Backdrop"]
        .into_iter()
        .map(|kind| json!({"item_id":"movie-1","kind":kind,"tag":format!("{kind}-revision")}))
        .collect()
}

fn resume(cursor: &Cursor) -> Cursor {
    serde_json::from_value(serde_json::to_value(cursor).unwrap()).unwrap()
}

#[test]
fn artwork_continuation_visits_every_identity_across_checkpoints() {
    let inputs = inputs();
    let mut cursor = Cursor {
        phase: 3,
        after: Some("movie-0".into()),
        ..Default::default()
    };
    let mut keys = Vec::new();
    let mut operations = Vec::new();
    for expected in &inputs {
        let (step, input) = ArtworkStep::next("movie-1".into(), &inputs, &mut cursor).unwrap();
        assert_eq!(&input, expected);
        let art = live::parse_artwork(&json!({"provider_input":input})).unwrap();
        keys.push(live::artwork_key(&art));
        operations.push(cursor.operation.clone());
        step.complete(&mut cursor);
        cursor = resume(&cursor);
        if keys.len() < inputs.len() {
            assert_eq!(cursor.after.as_deref(), Some("movie-0"));
            assert_eq!(cursor.art_index, keys.len());
        }
    }
    assert_eq!(cursor.after.as_deref(), Some("movie-1"));
    assert_eq!(cursor.art_index, 0);
    assert_eq!(cursor.phase, 3);
    assert_eq!(
        keys.iter().collect::<std::collections::BTreeSet<_>>().len(),
        4
    );
    assert_eq!(
        operations
            .iter()
            .collect::<std::collections::BTreeSet<_>>()
            .len(),
        4
    );
}

#[test]
fn artwork_unsettled_attempt_resumes_same_image_and_operation() {
    let inputs = inputs();
    for index in 0..inputs.len() {
        let mut cursor = Cursor {
            phase: 3,
            after: Some("movie-0".into()),
            art_index: index,
            ..Default::default()
        };
        let (_, first) = ArtworkStep::next("movie-1".into(), &inputs, &mut cursor).unwrap();
        let operation = cursor.operation.clone();
        // Retry, throttling, unauthorized, and failed work do not complete a step.
        cursor = resume(&cursor);
        let (step, retry) = ArtworkStep::next("movie-1".into(), &inputs, &mut cursor).unwrap();
        assert_eq!(retry, first);
        assert_eq!(cursor.operation, operation);
        assert_eq!(cursor.art_index, index);
        assert_eq!(cursor.after.as_deref(), Some("movie-0"));
        step.complete(&mut cursor);
        assert_eq!(cursor.art_index, (index + 1) % inputs.len());
    }
}

#[test]
fn artwork_ready_success_and_omission_share_input_bounded_completion() {
    let inputs = inputs();
    let mut cursor = Cursor::default();
    // Ready and omissions produce no input writes; a fetched image produces one.
    for output_count in [0, 1, 0, 1] {
        let (step, _) = ArtworkStep::next("movie-1".into(), &inputs, &mut cursor).unwrap();
        let before = cursor.art_index;
        let outputs = vec![(); output_count];
        step.complete(&mut cursor);
        cursor = resume(&cursor);
        if before < 3 {
            assert!(cursor.after.is_none());
            assert_eq!(cursor.art_index, before + 1);
            assert!(cursor.art_index >= outputs.len());
        } else {
            assert_eq!(cursor.after.as_deref(), Some("movie-1"));
            assert_eq!(cursor.art_index, 0);
        }
    }
}
