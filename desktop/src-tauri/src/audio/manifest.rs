use crate::audio::frame::{AudioChunkEnvelope, AudioCodec, AudioFrame, AudioPurpose, VadSegment};

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioSessionEnvelope {
    pub session_id: u64,
    pub source: AudioSource,
    pub started_at_ms: u64,
    pub sample_rate_hz: u32,
    pub chunks: Vec<AudioChunkEnvelope>,
    pub degraded: bool,
}

pub struct AudioChunkEnvelopeBuilder {
    session_id: u64,
    sequence_start: Option<u64>,
    purpose: AudioPurpose,
    codec: AudioCodec,
    frames: Vec<AudioFrame>,
}

impl AudioChunkEnvelopeBuilder {
    pub fn new(session_id: u64, purpose: AudioPurpose, codec: AudioCodec) -> Self {
        Self {
            session_id,
            sequence_start: None,
            purpose,
            codec,
            frames: Vec::new(),
        }
    }

    pub fn push(&mut self, frame: AudioFrame) {
        self.sequence_start = Some(match self.sequence_start {
            Some(sequence_start) => sequence_start.min(frame.sequence),
            None => frame.sequence,
        });
        self.frames.push(frame);
    }

    pub fn finish(mut self, vad_segments: Vec<VadSegment>) -> Option<AudioChunkEnvelope> {
        let sequence_start = self.sequence_start?;
        self.frames
            .sort_by_key(|frame| (frame.sequence, frame.start_ms, frame.duration_ms));

        AudioChunkEnvelope::from_frames(
            self.session_id,
            sequence_start,
            &self.frames,
            self.codec,
            vad_segments,
            self.purpose,
        )
    }
}

pub struct AudioSessionEnvelopeBuilder {
    session_id: u64,
    source: AudioSource,
    started_at_ms: u64,
    sample_rate_hz: u32,
    chunks: Vec<AudioChunkEnvelope>,
    degraded: bool,
}

impl AudioSessionEnvelopeBuilder {
    pub fn new(
        session_id: u64,
        source: AudioSource,
        started_at_ms: u64,
        sample_rate_hz: u32,
    ) -> Self {
        Self {
            session_id,
            source,
            started_at_ms,
            sample_rate_hz,
            chunks: Vec::new(),
            degraded: false,
        }
    }

    pub fn push_chunk(&mut self, chunk: AudioChunkEnvelope) {
        self.chunks.push(chunk);
    }

    pub fn mark_degraded(&mut self) {
        self.degraded = true;
    }

    pub fn finish(mut self) -> AudioSessionEnvelope {
        self.chunks
            .sort_by_key(|chunk| (chunk.sequence_start, chunk.start_ms, chunk.chunk_id.clone()));

        AudioSessionEnvelope {
            session_id: self.session_id,
            source: self.source,
            started_at_ms: self.started_at_ms,
            sample_rate_hz: self.sample_rate_hz,
            chunks: self.chunks,
            degraded: self.degraded,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AudioSource {
    Live,
    Recording,
}

#[cfg(test)]
mod tests {
    use super::{
        AudioChunkEnvelopeBuilder, AudioSessionEnvelope, AudioSessionEnvelopeBuilder, AudioSource,
    };
    use crate::audio::frame::{
        AudioChunkEnvelope, AudioCodec, AudioFrame, AudioPurpose, RetryMetadata, VadSegment,
    };
    use crate::audio::vad::VadKind;

    fn frame(
        session_id: u64,
        sequence: u64,
        start_ms: u64,
        duration_ms: u32,
        sample_rate_hz: u32,
    ) -> AudioFrame {
        AudioFrame {
            session_id,
            sequence,
            sample_rate_hz,
            channels: 1,
            start_ms,
            duration_ms,
            sample_count: 320,
        }
    }

    #[test]
    fn session_envelope_serializes_with_expected_field_names() {
        let session = AudioSessionEnvelope {
            session_id: 55,
            source: AudioSource::Live,
            started_at_ms: 1_000,
            sample_rate_hz: 16_000,
            chunks: vec![AudioChunkEnvelope {
                session_id: 55,
                chunk_id: "55-2-40".into(),
                sequence_start: 2,
                start_ms: 40,
                duration_ms: 40,
                sample_rate_hz: 16_000,
                codec: AudioCodec::PcmS16Le,
                vad_segments: vec![VadSegment {
                    start_ms: 40,
                    end_ms: 80,
                    kind: VadKind::Speech,
                    rms: 0.33,
                }],
                purpose: AudioPurpose::CaptureEnvelope,
                retry: RetryMetadata {
                    idempotency_key: "55-2-55-2-40".into(),
                    attempt: 1,
                    max_attempts: 1,
                },
            }],
            degraded: true,
        };

        let value = serde_json::to_value(&session).expect("session envelope should serialize");

        assert_eq!(value["sessionId"], 55);
        assert_eq!(value["source"], "live");
        assert_eq!(value["startedAtMs"], 1_000);
        assert_eq!(value["sampleRateHz"], 16_000);
        assert_eq!(value["degraded"], true);
        assert_eq!(value["chunks"][0]["chunkId"], "55-2-40");
    }

    #[test]
    fn chunk_builder_orders_contiguous_frames_by_sequence() {
        let mut builder =
            AudioChunkEnvelopeBuilder::new(7, AudioPurpose::CaptureEnvelope, AudioCodec::PcmS16Le);
        builder.push(frame(7, 12, 120, 20, 16_000));
        builder.push(frame(7, 11, 100, 20, 16_000));

        let vad_segments = vec![VadSegment {
            start_ms: 100,
            end_ms: 140,
            kind: VadKind::Speech,
            rms: 0.42,
        }];

        let envelope = builder
            .finish(vad_segments.clone())
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
    }

    #[test]
    fn chunk_builder_returns_none_for_empty_builders() {
        let builder =
            AudioChunkEnvelopeBuilder::new(7, AudioPurpose::LocalFallback, AudioCodec::PcmS16Le);

        assert!(builder.finish(Vec::new()).is_none());
    }

    #[test]
    fn chunk_builder_sets_retry_and_idempotency_fields() {
        let mut builder =
            AudioChunkEnvelopeBuilder::new(7, AudioPurpose::LocalFallback, AudioCodec::PcmS16Le);
        builder.push(frame(7, 11, 100, 20, 16_000));
        builder.push(frame(7, 12, 120, 20, 16_000));

        let envelope = builder
            .finish(vec![VadSegment {
                start_ms: 100,
                end_ms: 140,
                kind: VadKind::Speech,
                rms: 0.42,
            }])
            .expect("frames should build an envelope");

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
    fn session_builder_collects_chunks_and_marks_degraded() {
        let mut first_chunk_builder =
            AudioChunkEnvelopeBuilder::new(55, AudioPurpose::CaptureEnvelope, AudioCodec::PcmS16Le);
        first_chunk_builder.push(frame(55, 4, 80, 20, 16_000));
        first_chunk_builder.push(frame(55, 5, 100, 20, 16_000));
        let first_chunk = first_chunk_builder
            .finish(vec![VadSegment {
                start_ms: 80,
                end_ms: 120,
                kind: VadKind::Silence,
                rms: 0.0,
            }])
            .expect("first chunk should build");

        let mut second_chunk_builder =
            AudioChunkEnvelopeBuilder::new(55, AudioPurpose::CaptureEnvelope, AudioCodec::PcmS16Le);
        second_chunk_builder.push(frame(55, 2, 40, 20, 16_000));
        second_chunk_builder.push(frame(55, 3, 60, 20, 16_000));
        let second_chunk = second_chunk_builder
            .finish(vec![VadSegment {
                start_ms: 40,
                end_ms: 80,
                kind: VadKind::Speech,
                rms: 0.33,
            }])
            .expect("second chunk should build");

        let mut session_builder =
            AudioSessionEnvelopeBuilder::new(55, AudioSource::Live, 1_000, 16_000);
        session_builder.push_chunk(first_chunk);
        session_builder.push_chunk(second_chunk);
        session_builder.mark_degraded();

        let session = session_builder.finish();

        assert_eq!(session.session_id, 55);
        assert_eq!(session.source, AudioSource::Live);
        assert_eq!(session.started_at_ms, 1_000);
        assert_eq!(session.sample_rate_hz, 16_000);
        assert!(session.degraded);
        assert_eq!(session.chunks.len(), 2);
        assert_eq!(session.chunks[0].chunk_id, "55-2-40");
        assert_eq!(session.chunks[1].chunk_id, "55-4-40");
    }
}
