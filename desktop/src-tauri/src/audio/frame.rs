#[derive(Debug, Clone, PartialEq)]
pub struct AudioFrame {
    pub session_id: u64,
    pub sequence: u64,
    pub sample_rate_hz: u32,
    pub channels: u16,
    pub start_ms: u64,
    pub duration_ms: u32,
    pub sample_count: usize,
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
    pub session_id: u64,
    pub chunk_id: String,
    pub sequence_start: u64,
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
        session_id: u64,
        sequence_start: u64,
        frames: &[AudioFrame],
        codec: AudioCodec,
        vad_segments: Vec<VadSegment>,
        purpose: AudioPurpose,
    ) -> Option<Self> {
        let first = frames.first()?;
        let last = frames.last().expect("first frame implies last frame");
        let duration_ms = last
            .start_ms
            .saturating_add(u64::from(last.duration_ms))
            .saturating_sub(first.start_ms) as u32;
        let chunk_id = format!("{session_id}-{sequence_start}-{duration_ms}");

        Some(Self {
            session_id,
            chunk_id: chunk_id.clone(),
            sequence_start,
            start_ms: first.start_ms,
            duration_ms,
            sample_rate_hz: first.sample_rate_hz,
            codec,
            vad_segments,
            purpose,
            retry: RetryMetadata {
                idempotency_key: format!("{session_id}-{sequence_start}-{chunk_id}"),
                attempt: 1,
                max_attempts: 1,
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{
        AudioChunkEnvelope, AudioCodec, AudioFrame, AudioPurpose, RetryMetadata, VadSegment,
    };
    use crate::audio::vad::VadKind;

    fn frame(sequence: u64, start_ms: u64, duration_ms: u32, sample_count: usize) -> AudioFrame {
        AudioFrame {
            session_id: 7,
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
    fn from_frames_returns_none_for_empty_frame_lists() {
        let envelope = AudioChunkEnvelope::from_frames(
            7,
            10,
            &[],
            AudioCodec::PcmS16Le,
            Vec::new(),
            AudioPurpose::LocalFallback,
        );

        assert!(envelope.is_none());
    }

    #[test]
    fn from_frames_builds_deterministic_chunk_and_retry_metadata() {
        let frames = vec![frame(11, 100, 20, 320), frame(12, 120, 20, 320)];
        let vad_segments = vec![VadSegment {
            start_ms: 100,
            end_ms: 140,
            kind: VadKind::Speech,
            rms: 0.42,
        }];

        let envelope = AudioChunkEnvelope::from_frames(
            7,
            11,
            &frames,
            AudioCodec::PcmS16Le,
            vad_segments.clone(),
            AudioPurpose::CaptureEnvelope,
        )
        .expect("frames should build an envelope");

        assert_eq!(envelope.session_id, 7);
        assert_eq!(envelope.chunk_id, "7-11-40");
        assert_eq!(envelope.sequence_start, 11);
        assert_eq!(envelope.start_ms, 100);
        assert_eq!(envelope.duration_ms, 40);
        assert_eq!(envelope.sample_rate_hz, 16_000);
        assert_eq!(envelope.codec, AudioCodec::PcmS16Le);
        assert_eq!(envelope.vad_segments, vad_segments);
        assert_eq!(envelope.purpose, AudioPurpose::CaptureEnvelope);
        assert_eq!(
            envelope.retry,
            RetryMetadata {
                idempotency_key: "7-11-7-11-40".into(),
                attempt: 1,
                max_attempts: 1,
            }
        );
    }

    #[test]
    fn chunk_envelope_serializes_with_expected_field_names() {
        let envelope = AudioChunkEnvelope {
            session_id: 9,
            chunk_id: "9-4-20".into(),
            sequence_start: 4,
            start_ms: 80,
            duration_ms: 20,
            sample_rate_hz: 16_000,
            codec: AudioCodec::PcmS16Le,
            vad_segments: vec![VadSegment {
                start_ms: 80,
                end_ms: 100,
                kind: VadKind::Silence,
                rms: 0.0,
            }],
            purpose: AudioPurpose::LocalFallback,
            retry: RetryMetadata {
                idempotency_key: "9-4-9-4-20".into(),
                attempt: 1,
                max_attempts: 2,
            },
        };

        let value = serde_json::to_value(&envelope).expect("chunk envelope should serialize");

        assert_eq!(value["sessionId"], 9);
        assert_eq!(value["chunkId"], "9-4-20");
        assert_eq!(value["sequenceStart"], 4);
        assert_eq!(value["startMs"], 80);
        assert_eq!(value["durationMs"], 20);
        assert_eq!(value["sampleRateHz"], 16_000);
        assert_eq!(value["codec"], "pcm_s16_le");
        assert_eq!(value["purpose"], "localFallback");
        assert_eq!(value["retry"]["idempotencyKey"], "9-4-9-4-20");
        assert_eq!(value["retry"]["attempt"], 1);
        assert_eq!(value["retry"]["maxAttempts"], 2);
        assert_eq!(value["vadSegments"][0]["kind"], "silence");
    }
}
