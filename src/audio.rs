//! WAV construction and lightweight WAV/MP3 duration parsing.

const MP3_BITRATES: [[u32; 16]; 2] = [
    [
        0, 32, 40, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320, 0,
    ],
    [
        0, 8, 16, 24, 32, 40, 48, 56, 64, 80, 96, 112, 128, 144, 160, 0,
    ],
];
const MP3_SAMPLE_RATES: [[u32; 3]; 3] = [[11025, 12000, 8000], [0, 0, 0], [22050, 24000, 16000]];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WavInfo {
    pub sample_rate: u32,
    pub channels: u16,
    pub bits: u16,
    pub data_offset: usize,
    pub data_size: usize,
}

pub fn wav_info(data: &[u8]) -> Option<WavInfo> {
    if data.len() < 44 || &data[0..4] != b"RIFF" || &data[8..12] != b"WAVE" {
        return None;
    }
    let mut pos = 12usize;
    let mut sample_rate = 0u32;
    let mut channels = 0u16;
    let mut bits = 0u16;
    let mut data_offset = 0usize;
    let mut data_size = 0usize;

    while pos + 8 <= data.len() {
        let chunk_id = &data[pos..pos + 4];
        let size = u32::from_le_bytes(data[pos + 4..pos + 8].try_into().ok()?) as usize;
        let body = pos + 8;
        if chunk_id == b"fmt " && size >= 16 && body + 16 <= data.len() {
            channels = u16::from_le_bytes(data[body + 2..body + 4].try_into().ok()?);
            sample_rate = u32::from_le_bytes(data[body + 4..body + 8].try_into().ok()?);
            bits = u16::from_le_bytes(data[body + 14..body + 16].try_into().ok()?);
        } else if chunk_id == b"data" {
            data_offset = body;
            data_size = size.min(data.len().saturating_sub(body));
            if sample_rate != 0 && channels != 0 && bits != 0 {
                break;
            }
        }
        pos = body.saturating_add(size).saturating_add(size & 1);
    }

    if sample_rate == 0 || channels == 0 || bits == 0 {
        return None;
    }
    Some(WavInfo {
        sample_rate,
        channels,
        bits,
        data_offset,
        data_size,
    })
}

pub fn wav_pcm(data: &[u8]) -> Option<(Vec<u8>, u32)> {
    let info = wav_info(data)?;
    let start = info.data_offset;
    let end = start.saturating_add(info.data_size).min(data.len());
    Some((data[start..end].to_vec(), info.sample_rate))
}

pub fn build_wav(pcm: &[u8], sample_rate: u32) -> Vec<u8> {
    let channels = 1u16;
    let bits = 16u16;
    let byte_rate = sample_rate * u32::from(channels) * u32::from(bits) / 8;
    let block_align = channels * bits / 8;
    let data_size = pcm.len() as u32;

    let mut out = Vec::with_capacity(44 + pcm.len());
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_size).to_le_bytes());
    out.extend_from_slice(b"WAVE");
    out.extend_from_slice(b"fmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&channels.to_le_bytes());
    out.extend_from_slice(&sample_rate.to_le_bytes());
    out.extend_from_slice(&byte_rate.to_le_bytes());
    out.extend_from_slice(&block_align.to_le_bytes());
    out.extend_from_slice(&bits.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_size.to_le_bytes());
    out.extend_from_slice(pcm);
    out
}

pub fn pcm_f32_to_i16(samples: &[f32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(samples.len() * 2);
    for sample in samples {
        let clamped = sample.clamp(-1.0, 1.0);
        let value = (clamped * i16::MAX as f32).round() as i16;
        out.extend_from_slice(&value.to_le_bytes());
    }
    out
}

pub fn wav_duration_ms(data: &[u8]) -> Option<u32> {
    let info = wav_info(data)?;
    let frame_bytes = usize::from(info.channels) * usize::from(info.bits.max(1) / 8);
    if frame_bytes == 0 || info.sample_rate == 0 {
        return None;
    }
    let frames = info.data_size / frame_bytes;
    Some((frames as u64 * 1000 / u64::from(info.sample_rate)) as u32)
}

pub fn mp3_duration_ms(data: &[u8]) -> Option<u32> {
    if data.len() < 4 {
        return None;
    }
    let mut pos = 0usize;
    if data.len() >= 10 && &data[0..3] == b"ID3" {
        let size = ((usize::from(data[6] & 0x7f)) << 21)
            | ((usize::from(data[7] & 0x7f)) << 14)
            | ((usize::from(data[8] & 0x7f)) << 7)
            | usize::from(data[9] & 0x7f);
        pos = 10 + size;
    }

    let mut samples = 0u64;
    let mut sample_rate: Option<u32> = None;
    while pos + 4 <= data.len() {
        if data[pos] != 0xff || (data[pos + 1] & 0xe0) != 0xe0 {
            pos += 1;
            continue;
        }
        let version = (data[pos + 1] >> 3) & 0x03;
        let layer = (data[pos + 1] >> 1) & 0x03;
        if version == 1 || layer != 1 {
            pos += 1;
            continue;
        }
        let bitrate_index = usize::from((data[pos + 2] >> 4) & 0x0f);
        let rate_index = usize::from((data[pos + 2] >> 2) & 0x03);
        let padding = u32::from((data[pos + 2] >> 1) & 0x01);
        if bitrate_index == 0 || bitrate_index == 15 || rate_index == 3 {
            pos += 1;
            continue;
        }
        let table = if version == 3 { 0 } else { 1 };
        let bitrate = MP3_BITRATES[table][bitrate_index] * 1000;
        let rate_table = match version {
            3 => 0,
            2 => 2,
            _ => 1,
        };
        let rate = MP3_SAMPLE_RATES[rate_table][rate_index];
        if rate == 0 {
            pos += 1;
            continue;
        }
        if sample_rate.is_none() {
            sample_rate = Some(rate);
        }
        let (frame_length, frame_samples) = if version == 3 {
            ((144 * bitrate) / rate + padding, 1152u64)
        } else {
            ((72 * bitrate) / rate + padding, 576u64)
        };
        if frame_length <= 4 || pos + frame_length as usize > data.len() {
            pos += 1;
            continue;
        }
        samples += frame_samples;
        pos += frame_length as usize;
        if pos + 3 <= data.len() && &data[pos..pos + 3] == b"TAG" {
            break;
        }
    }

    let rate = sample_rate?;
    if samples == 0 {
        return None;
    }
    Some((samples * 1000 / u64::from(rate)) as u32)
}

pub fn duration_ms(data: &[u8], mime: &str) -> Option<u32> {
    let mime = mime.to_ascii_lowercase();
    if mime.contains("wav") {
        return wav_duration_ms(data);
    }
    if mime.contains("mpeg") || mime.contains("mp3") {
        return mp3_duration_ms(data);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wav_roundtrip_reports_duration() {
        let pcm = vec![0u8; 22050 * 2];
        let wav = build_wav(&pcm, 22050);
        let info = wav_info(&wav).unwrap();
        assert_eq!(info.sample_rate, 22050);
        assert_eq!(info.channels, 1);
        assert_eq!(info.bits, 16);
        assert_eq!(wav_duration_ms(&wav), Some(1000));
    }

    #[test]
    fn pcm_conversion_clamps() {
        let bytes = pcm_f32_to_i16(&[-2.0, 0.0, 2.0]);
        assert_eq!(i16::from_le_bytes([bytes[0], bytes[1]]), i16::MIN + 1);
        assert_eq!(i16::from_le_bytes([bytes[2], bytes[3]]), 0);
        assert_eq!(i16::from_le_bytes([bytes[4], bytes[5]]), i16::MAX);
    }

    #[test]
    fn rejects_non_wav() {
        assert!(wav_info(b"not a wav file").is_none());
        assert!(duration_ms(b"\xff\xfb\x90\x00", "audio/mpeg").is_none());
    }
}
