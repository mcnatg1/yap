pub fn downmix_to_mono(samples: &[f32], channels: usize) -> Vec<f32> {
    if channels == 0 {
        return Vec::new();
    }
    samples
        .chunks_exact(channels)
        .map(|frame| frame.iter().sum::<f32>() / channels as f32)
        .collect()
}

pub fn f32_to_i16_le_bytes(samples: &[f32]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(samples.len() * 2);
    for sample in samples {
        let value = if *sample <= -1.0 {
            i16::MIN
        } else if *sample >= 1.0 {
            i16::MAX
        } else {
            (*sample * i16::MAX as f32).round() as i16
        };
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    bytes
}

pub fn rms_level(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    let sum = samples.iter().map(|sample| sample * sample).sum::<f32>();
    (sum / samples.len() as f32).sqrt().clamp(0.0, 1.0)
}

// Ported from zachlatta/freeflow's LiveAudioLevelNormalizer (MIT).
pub struct AudioLevelNormalizer {
    noise_floor_db: f32,
    peak_ceiling_db: f32,
    display_level: f32,
}

impl AudioLevelNormalizer {
    const MINIMUM_RMS: f32 = 0.00001;
    const MIN_SPAN_DB: f32 = 18.0;
    const PEAK_HEADROOM_DB: f32 = 8.0;
    const SPEECH_GATE_MARGIN_DB: f32 = 3.0;
    const MINIMUM_VISIBLE_ACTIVE_LEVEL: f32 = 0.12;
    const NOISE_GATE_NORMALIZED_THRESHOLD: f32 = 0.06;
    const FLOOR_RISE_WINDOW_DB: f32 = 4.0;
    const FLOOR_FALL_BLEND: f32 = 0.12;
    const FLOOR_RISE_BLEND: f32 = 0.02;
    const PEAK_ATTACK_BLEND: f32 = 0.55;
    const PEAK_RELEASE_BLEND: f32 = 0.04;
    const DISPLAY_ATTACK_BLEND: f32 = 0.45;
    const DISPLAY_RELEASE_BLEND: f32 = 0.12;

    pub fn new() -> Self {
        Self {
            noise_floor_db: -55.0,
            peak_ceiling_db: -37.0,
            display_level: 0.0,
        }
    }

    pub fn normalized_level(&mut self, rms: f32) -> f32 {
        let level_db = 20.0 * rms.max(Self::MINIMUM_RMS).log10();

        self.update_noise_floor(level_db);
        self.update_peak_ceiling(level_db);

        let display_ceiling_db = self.peak_ceiling_db + Self::PEAK_HEADROOM_DB;
        let dynamic_span = (display_ceiling_db - self.noise_floor_db)
            .max(Self::MIN_SPAN_DB + Self::PEAK_HEADROOM_DB);
        let mut normalized = ((level_db - self.noise_floor_db) / dynamic_span).clamp(0.0, 1.0);
        let is_active_speech = level_db >= self.noise_floor_db + Self::SPEECH_GATE_MARGIN_DB;

        if normalized < Self::NOISE_GATE_NORMALIZED_THRESHOLD
            && level_db <= self.noise_floor_db + Self::SPEECH_GATE_MARGIN_DB
        {
            normalized = 0.0;
        } else if is_active_speech {
            normalized = normalized.max(Self::MINIMUM_VISIBLE_ACTIVE_LEVEL);
        }

        let blend = if normalized > self.display_level {
            Self::DISPLAY_ATTACK_BLEND
        } else {
            Self::DISPLAY_RELEASE_BLEND
        };
        self.display_level = mix(self.display_level, normalized, blend);
        self.display_level
    }

    fn update_noise_floor(&mut self, level_db: f32) {
        let ceiling_limited_level = level_db.min(self.peak_ceiling_db - Self::MIN_SPAN_DB);

        if ceiling_limited_level <= self.noise_floor_db {
            self.noise_floor_db = mix(
                self.noise_floor_db,
                ceiling_limited_level,
                Self::FLOOR_FALL_BLEND,
            );
        } else if ceiling_limited_level <= self.noise_floor_db + Self::FLOOR_RISE_WINDOW_DB {
            self.noise_floor_db = mix(
                self.noise_floor_db,
                ceiling_limited_level,
                Self::FLOOR_RISE_BLEND,
            );
        }
    }

    fn update_peak_ceiling(&mut self, level_db: f32) {
        let minimum_ceiling = self.noise_floor_db + Self::MIN_SPAN_DB;

        if level_db >= self.peak_ceiling_db {
            self.peak_ceiling_db = mix(self.peak_ceiling_db, level_db, Self::PEAK_ATTACK_BLEND);
        } else {
            self.peak_ceiling_db = mix(
                self.peak_ceiling_db,
                level_db.max(minimum_ceiling),
                Self::PEAK_RELEASE_BLEND,
            );
        }

        self.peak_ceiling_db = self.peak_ceiling_db.max(minimum_ceiling);
    }
}

impl Default for AudioLevelNormalizer {
    fn default() -> Self {
        Self::new()
    }
}

fn mix(current: f32, target: f32, blend: f32) -> f32 {
    current + (target - current) * blend
}

pub struct LinearResampler {
    source_rate: u32,
    target_rate: u32,
    cursor: f64,
}

impl LinearResampler {
    pub fn new(source_rate: u32, target_rate: u32) -> Self {
        Self {
            source_rate: source_rate.max(1),
            target_rate: target_rate.max(1),
            cursor: 0.0,
        }
    }

    pub fn push(&mut self, input: &[f32]) -> Vec<f32> {
        if input.is_empty() {
            return Vec::new();
        }
        if self.source_rate == self.target_rate {
            return input.to_vec();
        }
        let step = self.source_rate as f64 / self.target_rate as f64;
        let mut output = Vec::new();
        while self.cursor < input.len() as f64 {
            let base = self.cursor.floor() as usize;
            let frac = (self.cursor - base as f64) as f32;
            let a = input[base];
            let b = input.get(base + 1).copied().unwrap_or(a);
            output.push(a + (b - a) * frac);
            self.cursor += step;
        }
        self.cursor -= input.len() as f64;
        output
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preprocess_module_is_available() {}

    #[test]
    fn mono_downmix_averages_channels() {
        assert_eq!(downmix_to_mono(&[1.0, 3.0, 2.0, 4.0], 2), vec![2.0, 3.0]);
    }

    #[test]
    fn pcm_conversion_clamps_to_i16() {
        assert_eq!(
            f32_to_i16_le_bytes(&[-2.0, 0.0, 2.0]),
            vec![0, 128, 0, 0, 255, 127]
        );
    }

    #[test]
    fn linear_resample_can_downsample() {
        let mut resampler = LinearResampler::new(4, 2);
        assert_eq!(resampler.push(&[0.0, 1.0, 0.0, -1.0]), vec![0.0, 0.0]);
    }

    #[test]
    fn live_level_normalizer_lifts_speech_without_lifting_floor() {
        let mut normalizer = AudioLevelNormalizer::new();

        for _ in 0..12 {
            assert_eq!(normalizer.normalized_level(0.00001), 0.0);
        }

        let speech = normalizer.normalized_level(0.02);

        assert!(speech >= AudioLevelNormalizer::MINIMUM_VISIBLE_ACTIVE_LEVEL);
        assert!(speech <= 1.0);
    }
}
