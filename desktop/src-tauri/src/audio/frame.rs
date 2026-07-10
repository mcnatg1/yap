use std::sync::Arc;

use crate::audio::session::{SessionId, TrackId};

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioFrame {
    pub session_id: SessionId,
    pub track_id: TrackId,
    pub sequence: u64,
    pub sample_rate_hz: u32,
    pub channels: u16,
    pub start_ms: u64,
    pub duration_ms: u32,
    pub sample_count: usize,
}

#[derive(Debug, Clone)]
pub struct PreparedFrame {
    pub metadata: AudioFrame,
    pub samples: Arc<[f32]>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AudioCodec {
    PcmS16Le,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AudioPurpose {
    LocalFallback,
    CaptureEnvelope,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VadSegment {
    pub start_ms: u64,
    pub end_ms: u64,
    pub kind: crate::audio::vad::VadKind,
    pub rms: f32,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RetryMetadata {
    pub idempotency_key: String,
    pub attempt: u16,
    pub max_attempts: u16,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioChunkEnvelope {
    pub session_id: SessionId,
    pub track_id: TrackId,
    pub chunk_id: String,
    pub sequence_start: u64,
    pub sequence_end: u64,
    pub start_ms: u64,
    pub duration_ms: u32,
    pub sample_rate_hz: u32,
    pub codec: AudioCodec,
    pub vad_segments: Vec<VadSegment>,
    pub purpose: AudioPurpose,
    pub retry: RetryMetadata,
}

impl AudioFrame {
    pub fn duration_ms_from_samples(sample_count: usize, sample_rate_hz: u32) -> u32 {
        if sample_rate_hz == 0 {
            return 0;
        }

        ((sample_count as u128) * 1_000 / u128::from(sample_rate_hz)) as u32
    }

    pub fn end_ms(&self) -> u64 {
        self.start_ms.saturating_add(u64::from(self.duration_ms))
    }
}

impl AudioChunkEnvelope {
    pub fn from_frames(
        frames: &[AudioFrame],
        codec: AudioCodec,
        vad_segments: Vec<VadSegment>,
        purpose: AudioPurpose,
    ) -> Option<Self> {
        let first = frames.first()?;
        if frames
            .iter()
            .any(|frame| frame.session_id != first.session_id || frame.track_id != first.track_id)
        {
            return None;
        }
        let sequence_start = frames.iter().map(|frame| frame.sequence).min()?;
        let sequence_end = frames.iter().map(|frame| frame.sequence).max()?;
        let start_ms = frames.iter().map(|frame| frame.start_ms).min()?;
        let end_ms = frames.iter().map(AudioFrame::end_ms).max()?;
        let duration_ms = end_ms.saturating_sub(start_ms) as u32;
        let chunk_id = format!(
            "{}-{}-{sequence_start}-{sequence_end}-{duration_ms}",
            first.session_id, first.track_id
        );

        Some(Self {
            session_id: first.session_id.clone(),
            track_id: first.track_id.clone(),
            chunk_id: chunk_id.clone(),
            sequence_start,
            sequence_end,
            start_ms,
            duration_ms,
            sample_rate_hz: first.sample_rate_hz,
            codec,
            vad_segments,
            purpose,
            retry: RetryMetadata {
                idempotency_key: format!(
                    "{}-{sequence_start}-{sequence_end}-{chunk_id}",
                    first.session_id
                ),
                attempt: 1,
                max_attempts: 1,
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{
        AudioChunkEnvelope, AudioCodec, AudioFrame, AudioPurpose, PreparedFrame, VadSegment,
    };
    use crate::audio::{
        session::{SessionId, TrackId},
        vad::VadKind,
    };

    fn frame(sequence: u64, start_ms: u64, duration_ms: u32, sample_count: usize) -> AudioFrame {
        AudioFrame {
            session_id: SessionId::new("s-test").unwrap(),
            track_id: TrackId::new("mic-1").unwrap(),
            sequence,
            sample_rate_hz: 16_000,
            channels: 1,
            start_ms,
            duration_ms,
            sample_count,
        }
    }

    #[test]
    fn duration_ms_from_samples_uses_session_relative_sample_math() {
        assert_eq!(AudioFrame::duration_ms_from_samples(320, 16_000), 20);
        assert_eq!(AudioFrame::duration_ms_from_samples(16_000, 16_000), 1_000);
        assert_eq!(AudioFrame::duration_ms_from_samples(0, 16_000), 0);
    }

    #[test]
    fn duration_ms_from_samples_returns_zero_for_zero_sample_rate() {
        assert_eq!(AudioFrame::duration_ms_from_samples(320, 0), 0);
    }

    #[test]
    fn end_ms_uses_saturating_frame_coverage() {
        assert_eq!(frame(11, u64::MAX - 5, 10, 320).end_ms(), u64::MAX);
    }

    #[test]
    fn from_frames_returns_none_for_empty_or_mixed_track_lists() {
        assert!(AudioChunkEnvelope::from_frames(
            &[],
            AudioCodec::PcmS16Le,
            Vec::new(),
            AudioPurpose::LocalFallback,
        )
        .is_none());

        let mut mixed = vec![frame(1, 0, 20, 320), frame(2, 20, 20, 320)];
        mixed[1].track_id = TrackId::new("mic-2").unwrap();
        assert!(AudioChunkEnvelope::from_frames(
            &mixed,
            AudioCodec::PcmS16Le,
            Vec::new(),
            AudioPurpose::LocalFallback,
        )
        .is_none());
    }

    #[test]
    fn from_frames_builds_track_aware_chunk_and_retry_metadata() {
        let frames = vec![frame(11, 100, 20, 320), frame(12, 120, 20, 320)];
        let vad_segments = vec![VadSegment {
            start_ms: 100,
            end_ms: 140,
            kind: VadKind::Speech,
            rms: 0.42,
        }];

        let envelope = AudioChunkEnvelope::from_frames(
            &frames,
            AudioCodec::PcmS16Le,
            vad_segments.clone(),
            AudioPurpose::CaptureEnvelope,
        )
        .unwrap();

        assert_eq!(envelope.session_id.as_str(), "s-test");
        assert_eq!(envelope.track_id.as_str(), "mic-1");
        assert_eq!(envelope.sequence_start, 11);
        assert_eq!(envelope.sequence_end, 12);
        assert_eq!(envelope.start_ms, 100);
        assert_eq!(envelope.duration_ms, 40);
        assert_eq!(envelope.vad_segments, vad_segments);
        assert!(envelope.retry.idempotency_key.contains("mic-1"));
    }

    #[test]
    fn prepared_frames_keep_samples_out_of_serializable_metadata() {
        let metadata = frame(1, 0, 20, 320);
        let prepared = PreparedFrame {
            metadata: metadata.clone(),
            samples: std::sync::Arc::from([0.0_f32, 0.25_f32]),
        };

        let value = serde_json::to_value(metadata).unwrap();
        assert!(value.get("samples").is_none());
        assert_eq!(prepared.samples.len(), 2);
        assert_eq!(prepared.metadata.track_id.as_str(), "mic-1");
    }
}
