use crate::audio::frame::AudioChunkEnvelope;

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AudioSource {
    Live,
    Recording,
}

#[cfg(test)]
mod tests {
    use super::{AudioSessionEnvelope, AudioSource};
    use crate::audio::frame::{
        AudioChunkEnvelope, AudioCodec, AudioPurpose, RetryMetadata, VadSegment,
    };
    use crate::audio::vad::VadKind;

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
                    max_attempts: 3,
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
}
