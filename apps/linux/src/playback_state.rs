pub const MAX_MS: u64 = 922_337_203_685_477;

pub fn failure_message(code: &screenroom_client_core::PlaybackErrorCode) -> &'static str {
    use screenroom_client_core::PlaybackErrorCode as Error;
    match code {
        Error::ProfileRequired => "Choose a Jellyfin profile before playback.",
        Error::ProfileChanged => "Your profile changed. Reopen this title to continue.",
        Error::SourceUnavailable | Error::UpstreamUnavailable => {
            "The media source is unavailable. Close the player and try later."
        }
        Error::ItemUnavailable => "This title is not available to play right now.",
        Error::UnsupportedMedia => {
            "This media format is not supported by the available player plugins."
        }
        Error::TranscodingUnavailable => "Compatible transcoding is unavailable for this title.",
        Error::PlaybackAuthorizationUnavailable => "The gateway could not authorize playback.",
        Error::PlaybackConflict | Error::OperationConflict => {
            "A playback operation conflicts with this session. Check session state."
        }
        Error::SessionExpired => "This playback session expired. Reopen the title to continue.",
        Error::CapacityExhausted => "Playback capacity is full. Close the player and try later.",
        Error::Unspecified => "Playback is unavailable. Check session state or close the player.",
    }
}

pub fn session_blocked(session: &screenroom_client_core::PlaybackSession) -> bool {
    use screenroom_client_core::{PlaybackReportOutcome as Outcome, PlaybackState as State};
    !matches!(session.state, State::Ready | State::Playing | State::Paused)
        || session.transport.is_none()
        || session.failure_code.is_some()
        || session.cleanup_pending
        || session.pending_operation.is_some()
        || matches!(
            session.report_outcome,
            Outcome::Pending | Outcome::Uncertain | Outcome::Failed
        )
}

pub fn milliseconds(value: &str) -> Option<u64> {
    if value.is_empty() || value.len() > 20 || !value.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    value.parse::<u64>().ok().filter(|ms| *ms <= MAX_MS)
}

pub fn logical_position(media: u64, offset: u64) -> Option<u64> {
    media.checked_add(offset).filter(|ms| *ms <= MAX_MS)
}

pub fn media_position(logical: u64, offset: u64) -> Option<u64> {
    logical.checked_sub(offset)
}

pub fn clock(ms: u64) -> String {
    let seconds = ms / 1000;
    if seconds >= 3600 {
        format!(
            "{}:{:02}:{:02}",
            seconds / 3600,
            seconds / 60 % 60,
            seconds % 60
        )
    } else {
        format!("{}:{:02}", seconds / 60, seconds % 60)
    }
}

#[derive(Default)]
pub struct Timeline {
    pub offset: u64,
    pub position: u64,
    pub duration: Option<u64>,
    exit: Option<u64>,
}

#[derive(Debug, PartialEq)]
pub enum SeekAction {
    Native(u64),
    Gateway(u64),
}

pub fn transport_update(
    session: &screenroom_client_core::PlaybackSession,
    request_id: String,
    position: u64,
    choices: Option<screenroom_client_core::PlaybackTrackChoices>,
    fallback: bool,
) -> screenroom_client_core::UpdatePlaybackSessionRequest {
    screenroom_client_core::UpdatePlaybackSessionRequest {
        session_id: session.session_id.clone(),
        control_capability: session.control_capability.clone(),
        request_id,
        expected_transport_revision: session.transport_revision,
        position_ms: position.to_string(),
        track_choices: choices,
        compatible_transcode_fallback: fallback.then_some(true),
    }
}

impl Timeline {
    pub fn from_transport(
        transport: &screenroom_client_core::PlaybackTransport,
        saved_position: Option<u64>,
    ) -> Option<Self> {
        let offset = milliseconds(&transport.timeline_offset_ms)?;
        let position = saved_position.or_else(|| milliseconds(&transport.start_position_ms))?;
        let duration = match transport.duration_ms.as_deref() {
            Some(value) => Some(milliseconds(value)?),
            None => None,
        };
        media_position(position, offset)?;
        Some(Self::new(offset, position, duration))
    }
    pub fn seek(&self, logical: u64) -> Option<SeekAction> {
        if self.exit.is_some() || logical > MAX_MS || self.duration.is_some_and(|d| logical > d) {
            return None;
        }
        Some(match media_position(logical, self.offset) {
            Some(media) => SeekAction::Native(media),
            None => SeekAction::Gateway(logical),
        })
    }
    pub fn new(offset: u64, position: u64, duration: Option<u64>) -> Self {
        Self {
            offset,
            position,
            duration,
            exit: None,
        }
    }
    pub fn observe(&mut self, media: u64) -> Option<u64> {
        if let Some(exit) = self.exit {
            return Some(exit);
        }
        self.position = logical_position(media, self.offset)?;
        Some(self.position)
    }
    pub fn freeze(&mut self) -> u64 {
        *self.exit.get_or_insert(self.position)
    }
}

#[derive(Default)]
pub struct Reports {
    started: bool,
    sequence: u32,
    uncertain: bool,
}
impl Reports {
    pub fn uncertain(&self) -> bool {
        self.uncertain
    }
    pub fn fail(&mut self) {
        self.uncertain = true;
    }
    pub fn final_sequence(&self, accepted: u32) -> u32 {
        self.sequence.max(accepted).saturating_add(1)
    }
    pub fn next(&mut self, start: bool, accepted: u32) -> Option<u32> {
        if self.uncertain || start == self.started {
            return None;
        }
        self.sequence = self.sequence.max(accepted).checked_add(1)?;
        self.started |= start;
        Some(self.sequence)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pre_origin_seek_updates_gateway_at_exact_target_and_installs_new_timeline() {
        use screenroom_client_core::{PlaybackSession, PlaybackTransport};
        let mut timeline = Timeline::new(120_000, 160_000, Some(300_000));
        let SeekAction::Gateway(target) = timeline.seek(30_000).unwrap() else {
            panic!("pre-origin seek must negotiate a gateway transport");
        };
        let current = PlaybackSession {
            transport_revision: 3,
            ..Default::default()
        };
        let request = transport_update(&current, "exact-seek-request".into(), target, None, false);
        assert_eq!(request.position_ms, "30000");
        assert_eq!(request.expected_transport_revision, 3);
        assert_eq!(request.track_choices, None);
        assert_eq!(request.compatible_transcode_fallback, None);
        let exact_retry = request.clone();
        assert_eq!(exact_retry, request);
        timeline.position = target;
        assert_eq!(timeline.freeze(), 30_000);
        let transport = PlaybackTransport {
            revision: 4,
            timeline_offset_ms: "20000".into(),
            start_position_ms: request.position_ms,
            duration_ms: Some("300000".into()),
            ..Default::default()
        };
        let mut next = Timeline::from_transport(&transport, None).unwrap();
        assert_eq!(next.seek(30_000), Some(SeekAction::Native(10_000)));
        assert_eq!(next.observe(10_000), Some(30_000));
        assert_eq!(next.observe(15_000), Some(35_000));
        assert_eq!(next.freeze(), 35_000);
        assert_eq!(next.observe(0), Some(35_000));
    }

    #[test]
    fn pre_origin_seek_to_zero_and_seek_bounds_remain_logical() {
        let timeline = Timeline::new(120_000, 160_000, Some(300_000));
        assert_eq!(timeline.seek(0), Some(SeekAction::Gateway(0)));
        assert_eq!(timeline.seek(120_000), Some(SeekAction::Native(0)));
        assert_eq!(timeline.seek(300_000), Some(SeekAction::Native(180_000)));
        assert_eq!(timeline.seek(300_001), None);
    }
    #[test]
    fn uncertain_terminal_and_missing_transport_sessions_never_play() {
        use screenroom_client_core::{
            PlaybackReportOutcome as Outcome, PlaybackSession, PlaybackState as State,
            PlaybackTransport,
        };
        let ready = PlaybackSession {
            state: State::Ready,
            transport: Some(PlaybackTransport::default()),
            report_outcome: Outcome::NotSent,
            ..Default::default()
        };
        assert!(!session_blocked(&ready));
        for state in [
            State::Preparing,
            State::Stopping,
            State::Stopped,
            State::Expired,
            State::Failed,
            State::Uncertain,
            State::Unspecified,
        ] {
            assert!(session_blocked(&PlaybackSession {
                state,
                ..ready.clone()
            }));
        }
        for report_outcome in [Outcome::Pending, Outcome::Uncertain, Outcome::Failed] {
            assert!(session_blocked(&PlaybackSession {
                report_outcome,
                ..ready.clone()
            }));
        }
        assert!(session_blocked(&PlaybackSession {
            transport: None,
            ..ready.clone()
        }));
        assert!(session_blocked(&PlaybackSession {
            cleanup_pending: true,
            ..ready
        }));
    }

    #[test]
    fn offsets_apply_once_and_backward_seek_is_not_monotonic() {
        let mut timeline = Timeline {
            offset: 120_000,
            ..Default::default()
        };
        assert_eq!(timeline.observe(40_000), Some(160_000));
        assert_eq!(media_position(130_000, timeline.offset), Some(10_000));
        assert_eq!(timeline.observe(10_000), Some(130_000));
        assert_eq!(media_position(100_000, timeline.offset), None);
    }

    #[test]
    fn exit_position_survives_detach_and_late_events() {
        let mut timeline = Timeline {
            position: 80_000,
            ..Default::default()
        };
        assert_eq!(timeline.freeze(), 80_000);
        assert_eq!(timeline.observe(0), Some(80_000));
        assert_eq!(timeline.freeze(), 80_000);
    }

    #[test]
    fn checked_wide_timelines_and_labels() {
        assert_eq!(milliseconds("922337203685477"), Some(MAX_MS));
        for value in ["", "-1", "1.0", "1e5", "922337203685478"] {
            assert_eq!(milliseconds(value), None);
        }
        assert_eq!(logical_position(MAX_MS, 1), None);
        assert_eq!(clock(3_661_000), "1:01:01");
        assert_eq!(clock(61_000), "1:01");
    }
    #[test]
    fn unavailable_states_offer_friendly_actions_without_raw_provider_errors() {
        use screenroom_client_core::PlaybackErrorCode as Error;
        assert_eq!(
            failure_message(&Error::ProfileRequired),
            "Choose a Jellyfin profile before playback."
        );
        assert_eq!(
            failure_message(&Error::ItemUnavailable),
            "This title is not available to play right now."
        );
        assert!(!failure_message(&Error::UpstreamUnavailable).contains("http"));
    }

    #[test]
    fn reports_are_ordered_and_start_is_never_replayed_after_ambiguity() {
        let mut reports = Reports::default();
        assert_eq!(reports.next(false, 0), None);
        assert_eq!(reports.next(true, 0), Some(1));
        assert_eq!(reports.next(true, 1), None);
        assert_eq!(reports.next(false, 1), Some(2));
        reports.fail();
        assert!(reports.uncertain());
        assert_eq!(reports.next(true, 2), None);
        assert_eq!(reports.next(false, 2), None);
        assert_eq!(reports.final_sequence(2), 3);
    }

    #[test]
    fn reports_follow_sequence_not_position_and_reject_overflow() {
        let mut reports = Reports::default();
        assert_eq!(reports.next(true, 9), Some(10));
        assert_eq!(reports.next(false, 10), Some(11));
        assert_eq!(reports.next(false, u32::MAX), None);
    }
}
