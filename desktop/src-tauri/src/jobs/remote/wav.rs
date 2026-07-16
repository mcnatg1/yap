use std::{
    fs::File,
    io::{Read, Seek, SeekFrom},
};

const MAX_JOB_PCM_BYTES: u64 = 16_000 * 2 * 4 * 60 * 60;
pub(super) const MAX_WAV_CONTAINER_OVERHEAD_BYTES: u64 = 1024 * 1024;

pub(super) struct WavData {
    pub(super) data_offset: u64,
    pub(super) data_bytes: u64,
    pub(super) source_bytes: u64,
}

pub(super) fn inspect_pcm_wav(source: &mut File) -> Result<WavData, String> {
    let length = source
        .metadata()
        .map_err(|error| format!("failed to inspect imported WAV: {error}"))?
        .len();
    source
        .seek(SeekFrom::Start(0))
        .map_err(|error| format!("failed to seek imported WAV: {error}"))?;
    let mut header = [0_u8; 12];
    source
        .read_exact(&mut header)
        .map_err(|_| "imported recording is shorter than a WAV header".to_string())?;
    if &header[0..4] != b"RIFF" || &header[8..12] != b"WAVE" {
        return Err("Phase 5 currently accepts canonical RIFF/WAVE input only".into());
    }
    let declared_end = u64::from(u32::from_le_bytes(header[4..8].try_into().unwrap())) + 8;
    if declared_end < 44 {
        return Err("imported WAV has an invalid RIFF length".into());
    }
    if declared_end != length {
        return Err("imported WAV file length does not match its RIFF boundary".into());
    }
    let mut format_valid = false;
    let mut data = None;
    let mut position = 12_u64;
    while position
        .checked_add(8)
        .is_some_and(|end| end <= declared_end)
    {
        source
            .seek(SeekFrom::Start(position))
            .map_err(|error| format!("failed to seek WAV chunk: {error}"))?;
        let mut chunk_header = [0_u8; 8];
        source
            .read_exact(&mut chunk_header)
            .map_err(|_| "imported WAV chunk header is truncated".to_string())?;
        let chunk_size = u64::from(u32::from_le_bytes(chunk_header[4..8].try_into().unwrap()));
        let chunk_start = position + 8;
        let chunk_end = chunk_start
            .checked_add(chunk_size)
            .ok_or_else(|| "imported WAV chunk length overflowed".to_string())?;
        if chunk_end > declared_end || chunk_end > length {
            return Err("imported WAV chunk exceeds the RIFF boundary".into());
        }
        if &chunk_header[0..4] == b"fmt " {
            if chunk_size < 16 {
                return Err("imported WAV format chunk is truncated".into());
            }
            let mut format = [0_u8; 16];
            source
                .read_exact(&mut format)
                .map_err(|_| "imported WAV format chunk is truncated".to_string())?;
            format_valid = u16::from_le_bytes(format[0..2].try_into().unwrap()) == 1
                && u16::from_le_bytes(format[2..4].try_into().unwrap()) == 1
                && u32::from_le_bytes(format[4..8].try_into().unwrap()) == 16_000
                && u32::from_le_bytes(format[8..12].try_into().unwrap()) == 32_000
                && u16::from_le_bytes(format[12..14].try_into().unwrap()) == 2
                && u16::from_le_bytes(format[14..16].try_into().unwrap()) == 16;
        } else if &chunk_header[0..4] == b"data" {
            data = Some(WavData {
                data_offset: chunk_start,
                data_bytes: chunk_size,
                source_bytes: declared_end,
            });
        }
        position = chunk_end
            .checked_add(chunk_size % 2)
            .ok_or_else(|| "imported WAV padding overflowed".to_string())?;
    }
    if !format_valid {
        return Err("Phase 5 requires mono signed PCM16 WAV at 16 kHz".into());
    }
    let data = data.ok_or_else(|| "imported WAV has no data chunk".to_string())?;
    validate_pcm_data_bytes(data.data_bytes)?;
    let container_overhead = data
        .source_bytes
        .checked_sub(data.data_bytes)
        .ok_or_else(|| "imported WAV data exceeds its container".to_string())?;
    if container_overhead > MAX_WAV_CONTAINER_OVERHEAD_BYTES {
        return Err("imported WAV container metadata is too large".into());
    }
    Ok(data)
}

pub(super) fn validate_pcm_data_bytes(data_bytes: u64) -> Result<(), String> {
    if data_bytes == 0 || !data_bytes.is_multiple_of(2) {
        return Err("imported WAV audio must contain whole PCM16 samples".into());
    }
    if data_bytes > MAX_JOB_PCM_BYTES {
        return Err("Phase 5 accepts at most four hours of PCM audio per recording".into());
    }
    Ok(())
}
