use std::path::Path;
use std::time::Instant;

use yap_desktop_lib::audio::preprocess::{downmix_to_mono, LinearResampler};
use yap_desktop_lib::live::stream::{self, LiveStreamEngine};
use yap_desktop_lib::stt::parity::word_error_rate;

fn main() {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    if args.is_empty() {
        eprintln!("usage: cargo run --example nemotron_profile -- <clip.wav> [reference.txt]");
        std::process::exit(2);
    }

    let wav = read_wav(Path::new(&args[0])).unwrap_or_else(|err| {
        eprintln!("failed to read wav: {err}");
        std::process::exit(1);
    });
    let audio_ms = wav.samples.len() as u64 * 1000 / 16_000;

    let load_started = Instant::now();
    let mut engine = LiveStreamEngine::new().unwrap_or_else(|err| {
        eprintln!("failed to load Nemotron: {}", err.user_message());
        std::process::exit(1);
    });
    let load_ms = load_started.elapsed().as_millis();

    let chunk = stream::chunk_samples();
    let mut final_text = String::new();
    let mut first_text_ms = None;
    let mut chunks = 0usize;
    let decode_started = Instant::now();

    for samples in wav.samples.chunks(chunk) {
        chunks += 1;
        if let Some(text) = engine.accept_samples(samples) {
            if first_text_ms.is_none() {
                first_text_ms = Some(decode_started.elapsed().as_millis());
            }
            final_text = text;
        }
    }
    if let Some(text) = engine.finish() {
        if first_text_ms.is_none() {
            first_text_ms = Some(decode_started.elapsed().as_millis());
        }
        final_text = text;
    }

    let decode_ms = decode_started.elapsed().as_millis();
    let rtf = decode_ms as f64 / audio_ms.max(1) as f64;

    println!("model=Nemotron 3.5 ASR Streaming 0.6B INT8");
    println!("input={}", args[0]);
    println!("source_rate={} channels={}", wav.source_rate, wav.channels);
    println!("audio_ms={audio_ms}");
    println!("chunk_ms={}", yap_desktop_lib::stt::nemotron::CHUNK_MS);
    println!("chunks={chunks}");
    println!("load_ms={load_ms}");
    println!("decode_ms={decode_ms}");
    println!("rtf={rtf:.3}");
    println!(
        "first_text_ms={}",
        first_text_ms
            .map(|value| value.to_string())
            .unwrap_or_else(|| "none".into())
    );

    if let Some(reference_path) = args.get(1) {
        let reference = std::fs::read_to_string(reference_path).unwrap_or_default();
        let wer = word_error_rate(&reference, &final_text) * 100.0;
        println!("wer={wer:.2}");
    }

    println!("transcript={}", final_text.trim());
}

struct WavAudio {
    source_rate: u32,
    channels: usize,
    samples: Vec<f32>,
}

fn read_wav(path: &Path) -> Result<WavAudio, String> {
    let bytes = std::fs::read(path).map_err(|err| err.to_string())?;
    if bytes.get(0..4) != Some(b"RIFF") || bytes.get(8..12) != Some(b"WAVE") {
        return Err("expected RIFF/WAVE file".into());
    }

    let mut cursor = 12usize;
    let mut fmt = None;
    let mut data = None;

    while cursor + 8 <= bytes.len() {
        let id = &bytes[cursor..cursor + 4];
        let len = u32::from_le_bytes(bytes[cursor + 4..cursor + 8].try_into().unwrap()) as usize;
        cursor += 8;
        if cursor + len > bytes.len() {
            return Err("truncated wav chunk".into());
        }
        match id {
            b"fmt " => fmt = Some(parse_fmt(&bytes[cursor..cursor + len])?),
            b"data" => data = Some(&bytes[cursor..cursor + len]),
            _ => {}
        }
        cursor += len + (len % 2);
    }

    let fmt = fmt.ok_or_else(|| "missing fmt chunk".to_string())?;
    let data = data.ok_or_else(|| "missing data chunk".to_string())?;
    let interleaved = decode_samples(data, fmt.audio_format, fmt.bits_per_sample)?;
    let mono = downmix_to_mono(&interleaved, fmt.channels as usize);
    let samples = if fmt.sample_rate == 16_000 {
        mono
    } else {
        LinearResampler::new(fmt.sample_rate, 16_000).push(&mono)
    };

    Ok(WavAudio {
        source_rate: fmt.sample_rate,
        channels: fmt.channels as usize,
        samples,
    })
}

struct WavFmt {
    audio_format: u16,
    channels: u16,
    sample_rate: u32,
    bits_per_sample: u16,
}

fn parse_fmt(bytes: &[u8]) -> Result<WavFmt, String> {
    if bytes.len() < 16 {
        return Err("fmt chunk too short".into());
    }
    Ok(WavFmt {
        audio_format: u16::from_le_bytes(bytes[0..2].try_into().unwrap()),
        channels: u16::from_le_bytes(bytes[2..4].try_into().unwrap()),
        sample_rate: u32::from_le_bytes(bytes[4..8].try_into().unwrap()),
        bits_per_sample: u16::from_le_bytes(bytes[14..16].try_into().unwrap()),
    })
}

fn decode_samples(
    data: &[u8],
    audio_format: u16,
    bits_per_sample: u16,
) -> Result<Vec<f32>, String> {
    match (audio_format, bits_per_sample) {
        (1, 16) => Ok(data
            .chunks_exact(2)
            .map(|chunk| i16::from_le_bytes(chunk.try_into().unwrap()) as f32 / i16::MAX as f32)
            .collect()),
        (1, 24) => Ok(data
            .chunks_exact(3)
            .map(|chunk| {
                let value = ((chunk[0] as i32) << 8)
                    | ((chunk[1] as i32) << 16)
                    | ((chunk[2] as i32) << 24);
                (value >> 8) as f32 / 8_388_607.0
            })
            .collect()),
        (1, 32) => Ok(data
            .chunks_exact(4)
            .map(|chunk| i32::from_le_bytes(chunk.try_into().unwrap()) as f32 / i32::MAX as f32)
            .collect()),
        (3, 32) => Ok(data
            .chunks_exact(4)
            .map(|chunk| f32::from_le_bytes(chunk.try_into().unwrap()).clamp(-1.0, 1.0))
            .collect()),
        _ => Err(format!(
            "unsupported wav format={audio_format} bits={bits_per_sample}"
        )),
    }
}
