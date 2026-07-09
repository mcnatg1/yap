#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VadKind {
    Speech,
    Silence,
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VadDecision {
    pub kind: VadKind,
    pub rms: f32,
    pub threshold: f32,
    pub start_ms: u64,
    pub end_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EnergyVadConfig {
    pub speech_rms_threshold: f32,
    pub tail_padding_ms: u32,
}

pub fn classify_energy(
    samples: &[f32],
    sample_rate_hz: u32,
    start_ms: u64,
    config: EnergyVadConfig,
) -> VadDecision {
    let rms = crate::audio::preprocess::rms_level(samples);
    let invalid = || VadDecision {
        kind: VadKind::Error,
        rms,
        threshold: config.speech_rms_threshold,
        start_ms,
        end_ms: start_ms,
    };

    if sample_rate_hz == 0 {
        return invalid();
    }

    let samples_len = match u128::try_from(samples.len()) {
        Ok(value) => value,
        Err(_) => return invalid(),
    };
    let sample_rate_hz = u128::from(sample_rate_hz);
    let duration_ms = (samples_len * 1_000) / sample_rate_hz;
    let base_end_ms = match u64::try_from(u128::from(start_ms) + duration_ms) {
        Ok(value) => value,
        Err(_) => return invalid(),
    };

    let is_speech = rms >= config.speech_rms_threshold;
    let end_ms = if is_speech {
        match base_end_ms.checked_add(u64::from(config.tail_padding_ms)) {
            Some(value) => value,
            None => return invalid(),
        }
    } else {
        base_end_ms
    };

    if end_ms < start_ms {
        return invalid();
    }

    VadDecision {
        kind: if is_speech {
            VadKind::Speech
        } else {
            VadKind::Silence
        },
        rms,
        threshold: config.speech_rms_threshold,
        start_ms,
        end_ms,
    }
}

#[cfg(test)]
mod tests {
    use super::{classify_energy, EnergyVadConfig, VadDecision, VadKind};

    #[test]
    fn vad_kind_serializes_with_snake_case_names() {
        assert_eq!(
            serde_json::to_string(&VadKind::Speech).expect("speech should serialize"),
            "\"speech\""
        );
        assert_eq!(
            serde_json::to_string(&VadKind::Silence).expect("silence should serialize"),
            "\"silence\""
        );
        assert_eq!(
            serde_json::to_string(&VadKind::Error).expect("error should serialize"),
            "\"error\""
        );
    }

    #[test]
    fn classify_energy_marks_speech_and_applies_tail_padding() {
        let decision = classify_energy(
            &[0.5, -0.5, 0.5, -0.5],
            4,
            100,
            EnergyVadConfig {
                speech_rms_threshold: 0.2,
                tail_padding_ms: 25,
            },
        );

        assert_eq!(
            decision,
            VadDecision {
                kind: VadKind::Speech,
                rms: 0.5,
                threshold: 0.2,
                start_ms: 100,
                end_ms: 1_125,
            }
        );
    }

    #[test]
    fn classify_energy_marks_silence_without_padding() {
        let decision = classify_energy(
            &[0.01, -0.01, 0.01, -0.01],
            4,
            200,
            EnergyVadConfig {
                speech_rms_threshold: 0.2,
                tail_padding_ms: 25,
            },
        );

        assert_eq!(
            decision,
            VadDecision {
                kind: VadKind::Silence,
                rms: 0.01,
                threshold: 0.2,
                start_ms: 200,
                end_ms: 1_200,
            }
        );
    }

    #[test]
    fn classify_energy_marks_invalid_sample_rate_as_error() {
        let decision = classify_energy(
            &[0.5, -0.5],
            0,
            300,
            EnergyVadConfig {
                speech_rms_threshold: 0.2,
                tail_padding_ms: 25,
            },
        );

        assert_eq!(
            decision,
            VadDecision {
                kind: VadKind::Error,
                rms: 0.5,
                threshold: 0.2,
                start_ms: 300,
                end_ms: 300,
            }
        );
    }
}
