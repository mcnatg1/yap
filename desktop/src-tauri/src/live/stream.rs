use std::path::Path;
use std::time::{Duration, Instant};

use sherpa_onnx::{OnlineRecognizer, OnlineRecognizerConfig, OnlineStream};

use crate::stt::error::SttError;

const SAMPLE_RATE: i32 = 16_000;
const TAIL_SILENCE: Duration = Duration::from_millis(500);

pub struct LiveStreamEngine {
    recognizer: OnlineRecognizer,
    stream: OnlineStream,
    last_text: String,
}

impl LiveStreamEngine {
    pub fn new() -> Result<Self, SttError> {
        let paths = crate::stt::nemotron::resolve_model()?;
        let started = Instant::now();
        let recognizer = OnlineRecognizer::create(&recognizer_config(&paths))
            .ok_or(SttError::SidecarUnreachable)?;
        crate::stt::log_stt_timed(
            "nemotron.load",
            started.elapsed(),
            crate::stt::nemotron::MODEL_LABEL,
        );
        let stream = recognizer.create_stream();
        Ok(Self {
            recognizer,
            stream,
            last_text: String::new(),
        })
    }

    pub fn reset(&mut self) {
        self.stream = self.recognizer.create_stream();
        self.last_text.clear();
    }

    pub fn accept_samples(&mut self, samples: &[f32]) -> Option<String> {
        if samples.is_empty() {
            return None;
        }
        self.stream.accept_waveform(SAMPLE_RATE, samples);
        self.decode_ready();
        self.changed_text()
    }

    pub fn finish(&mut self) -> Option<String> {
        let tail = vec![0.0; silence_samples(TAIL_SILENCE)];
        self.stream.accept_waveform(SAMPLE_RATE, &tail);
        self.stream.input_finished();
        self.decode_ready();
        self.changed_text()
            .or_else(|| (!self.last_text.is_empty()).then(|| self.last_text.clone()))
    }

    fn decode_ready(&self) {
        while self.recognizer.is_ready(&self.stream) {
            self.recognizer.decode(&self.stream);
        }
    }

    fn changed_text(&mut self) -> Option<String> {
        let text = self
            .recognizer
            .get_result(&self.stream)?
            .text
            .trim()
            .to_string();
        if text.is_empty() || text == self.last_text {
            return None;
        }
        self.last_text = text.clone();
        Some(text)
    }
}

pub fn chunk_samples() -> usize {
    (SAMPLE_RATE as u64 * crate::stt::nemotron::CHUNK_MS / 1000) as usize
}

pub fn silence_samples(duration: Duration) -> usize {
    (SAMPLE_RATE as u128 * duration.as_millis() / 1000) as usize
}

fn recognizer_config(paths: &crate::stt::nemotron::NemotronPaths) -> OnlineRecognizerConfig {
    let mut config = OnlineRecognizerConfig::default();
    config.model_config.transducer.encoder = Some(path_string(&paths.encoder));
    config.model_config.transducer.decoder = Some(path_string(&paths.decoder));
    config.model_config.transducer.joiner = Some(path_string(&paths.joiner));
    config.model_config.tokens = Some(path_string(&paths.tokens));
    config.model_config.num_threads = crate::stt::nemotron::NUM_THREADS;
    config.model_config.provider = Some("cpu".into());
    config.model_config.model_type = Some("nemo_transducer".into());
    config.decoding_method = Some("greedy_search".into());
    config
}

fn path_string(path: &Path) -> String {
    path.to_string_lossy().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stream_chunk_matches_pinned_nemotron_export() {
        assert_eq!(chunk_samples(), 17_920);
    }

    #[test]
    fn tail_silence_is_bounded() {
        assert_eq!(silence_samples(Duration::from_millis(500)), 8_000);
    }

    #[test]
    fn config_uses_nemotron_transducer_on_cpu() {
        let paths = crate::stt::nemotron::NemotronPaths {
            encoder: "C:/models/encoder.int8.onnx".into(),
            decoder: "C:/models/decoder.int8.onnx".into(),
            joiner: "C:/models/joiner.int8.onnx".into(),
            tokens: "C:/models/tokens.txt".into(),
        };
        let config = recognizer_config(&paths);
        assert_eq!(
            config.model_config.model_type.as_deref(),
            Some("nemo_transducer")
        );
        assert_eq!(config.model_config.provider.as_deref(), Some("cpu"));
        assert_eq!(
            config.model_config.num_threads,
            crate::stt::nemotron::NUM_THREADS
        );
        assert_eq!(config.decoding_method.as_deref(), Some("greedy_search"));
    }
}
