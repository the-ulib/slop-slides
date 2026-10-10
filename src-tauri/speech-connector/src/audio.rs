//! Bounded PCM16 WAV decoding and deterministic host normalization.
use crate::{Error, Result};
pub struct Pcm {
    pub samples: Vec<i16>,
    pub sample_rate: u32,
}
pub fn encode(pcm: &[i16], rate: u32) -> Vec<u8> {
    let bytes = (pcm.len() * 2) as u32;
    let mut wav = Vec::with_capacity(bytes as usize + 44);
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(bytes + 36).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16u32.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes());
    wav.extend_from_slice(&rate.to_le_bytes());
    wav.extend_from_slice(&(rate * 2).to_le_bytes());
    wav.extend_from_slice(&2u16.to_le_bytes());
    wav.extend_from_slice(&16u16.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&bytes.to_le_bytes());
    for sample in pcm {
        wav.extend_from_slice(&sample.to_le_bytes());
    }
    wav
}
pub fn decode(wav: &[u8]) -> Result<Pcm> {
    let bad = || Error::msg("Provider returned invalid or unsupported PCM16 WAV audio.");
    if wav.len() < 44
        || wav.len() > 600 * 96000 * 4 + 4096
        || &wav[..4] != b"RIFF"
        || &wav[8..12] != b"WAVE"
        || u32::from_le_bytes(wav[4..8].try_into().unwrap()) as usize + 8 != wav.len()
    {
        return Err(bad());
    }
    let mut offset = 12;
    let mut format = None;
    let mut data = None;
    while offset + 8 <= wav.len() {
        let kind = &wav[offset..offset + 4];
        let size = u32::from_le_bytes(wav[offset + 4..offset + 8].try_into().unwrap()) as usize;
        offset += 8;
        let end = offset
            .checked_add(size)
            .filter(|n| *n <= wav.len())
            .ok_or_else(bad)?;
        let chunk = &wav[offset..end];
        if kind == b"fmt " {
            if format.is_some() || size < 16 {
                return Err(bad());
            }
            let channels = u16::from_le_bytes(chunk[2..4].try_into().unwrap());
            let rate = u32::from_le_bytes(chunk[4..8].try_into().unwrap());
            if chunk[..2] != [1, 0]
                || !(1..=2).contains(&channels)
                || !(8000..=96000).contains(&rate)
                || chunk[14..16] != [16, 0]
                || u16::from_le_bytes(chunk[12..14].try_into().unwrap()) != channels * 2
                || u32::from_le_bytes(chunk[8..12].try_into().unwrap())
                    != rate * channels as u32 * 2
            {
                return Err(bad());
            }
            format = Some((channels, rate));
        }
        if kind == b"data" {
            if data.is_some() || size == 0 {
                return Err(bad());
            }
            data = Some(chunk);
        }
        offset = end + size % 2;
    }
    if offset != wav.len() {
        return Err(bad());
    }
    let (channels, rate) = format.ok_or_else(bad)?;
    let data = data.ok_or_else(bad)?;
    if data.len() % (channels as usize * 2) != 0
        || data.len() / (channels as usize * 2) > 600 * rate as usize
    {
        return Err(bad());
    }
    let samples = data
        .chunks_exact(channels as usize * 2)
        .map(|frame| {
            let sum: i32 = frame
                .chunks_exact(2)
                .map(|b| i16::from_le_bytes([b[0], b[1]]) as i32)
                .sum();
            (sum / channels as i32) as i16
        })
        .collect();
    Ok(Pcm {
        samples,
        sample_rate: rate,
    })
}
pub fn normalize(pcm: Pcm) -> Result<Vec<i16>> {
    if !(8000..=96000).contains(&pcm.sample_rate)
        || pcm.samples.is_empty()
        || pcm.samples.len() > 600 * pcm.sample_rate as usize
    {
        return Err(Error::msg("Invalid PCM rate/duration."));
    }
    if pcm.sample_rate == 24000 {
        return Ok(pcm.samples);
    }
    // The first non-Qwen fixture is 16 kHz. Downsampling needs a qualified
    // antialiasing implementation before adding a provider that returns >24 kHz.
    if pcm.sample_rate > 24000 {
        return Err(Error::msg(
            "This provider's sample rate requires an unsupported downsampler.",
        ));
    }
    let count = (pcm.samples.len() as u64 * 24000 / pcm.sample_rate as u64) as usize;
    if count == 0 || count > 600 * 24000 {
        return Err(Error::msg("Invalid recording duration."));
    }
    let mut out = Vec::with_capacity(count);
    for n in 0..count {
        let position = n as f64 * pcm.sample_rate as f64 / 24000.0;
        let a = position.floor() as usize;
        let b = (a + 1).min(pcm.samples.len() - 1);
        let fraction = position - a as f64;
        out.push(
            (pcm.samples[a] as f64 * (1.0 - fraction) + pcm.samples[b] as f64 * fraction).round()
                as i16,
        );
    }
    Ok(out)
}

/// Reference-only resampling: windowed sinc low-pass prevents downsampling aliases.
/// The existing provider normalization policy and stock take bytes are unchanged.
pub fn reference_pcm(pcm: Pcm) -> Result<Vec<i16>> {
    if pcm.samples.is_empty()
        || !(8000..=96000).contains(&pcm.sample_rate)
        || pcm.samples.len() > 30 * pcm.sample_rate as usize
    {
        return Err(Error::msg("Invalid reference recording."));
    }
    if pcm.sample_rate == 24000 {
        return Ok(pcm.samples);
    }
    let count = pcm.samples.len() as u64 * 24000 / pcm.sample_rate as u64;
    let cutoff = (24000.0 / pcm.sample_rate as f64).min(1.0) * 0.9;
    let mut out = Vec::with_capacity(count as usize);
    for n in 0..count {
        let position = n as f64 * pcm.sample_rate as f64 / 24000.0;
        let center = position.floor() as i64;
        let mut sum = 0.0;
        let mut weight = 0.0;
        for tap in -32..=32 {
            let index = center + tap;
            if index < 0 || index >= pcm.samples.len() as i64 {
                continue;
            }
            let distance = position - index as f64;
            if distance.abs() > 32.0 {
                continue;
            }
            let x = std::f64::consts::PI * distance * cutoff;
            let sinc = if x.abs() < 1e-10 { 1.0 } else { x.sin() / x };
            let w = cutoff * sinc * (0.5 + 0.5 * (std::f64::consts::PI * distance / 32.0).cos());
            sum += pcm.samples[index as usize] as f64 * w;
            weight += w;
        }
        out.push((sum / weight).round().clamp(-32768.0, 32767.0) as i16);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reference_resampling_keeps_duration_and_removes_above_nyquist_energy() {
        let sine = |hz: f64| Pcm {
            sample_rate: 48000,
            samples: (0..48000)
                .map(|n| ((n as f64 * hz * std::f64::consts::TAU / 48000.0).sin() * 10000.0) as i16)
                .collect(),
        };
        let audible = reference_pcm(sine(1000.0)).unwrap();
        let alias = reference_pcm(sine(18000.0)).unwrap();
        assert_eq!(audible.len(), 24000);
        assert_eq!(alias.len(), 24000);
        let rms = |pcm: &[i16]| {
            (pcm[100..pcm.len() - 100]
                .iter()
                .map(|n| (*n as f64).powi(2))
                .sum::<f64>()
                / (pcm.len() - 200) as f64)
                .sqrt()
        };
        assert!((rms(&audible) - 7070.0).abs() < 50.0);
        assert!(rms(&alias) < 20.0);
        let crossings = audible.windows(2).filter(|p| p[0] <= 0 && p[1] > 0).count();
        assert!((crossings as i32 - 1000).abs() <= 1);
    }
    #[test]
    fn normalizes_duration_and_rejects_bad_headers() {
        let bytes = encode(&vec![1200; 16000], 16000);
        let pcm = normalize(decode(&bytes).unwrap()).unwrap();
        assert_eq!(pcm.len(), 24000);
        assert!(pcm.iter().all(|n| *n == 1200));
        let mut bad = bytes.clone();
        bad[32] = 9;
        assert!(decode(&bad).is_err());
        let mut bad = bytes.clone();
        bad.push(0);
        assert!(decode(&bad).is_err());
        assert!(decode(&bytes[..30]).is_err());
        assert!(decode(&encode(&[], 24000)).is_err());
        assert!(normalize(decode(&encode(&[1, 2, 3], 48000)).unwrap()).is_err());
    }
    #[test]
    fn downmixes_stereo_and_bounds_chunk_lengths() {
        let mut wav = encode(&[1000, 3000, -1000, -3000], 16000);
        wav[22..24].copy_from_slice(&2u16.to_le_bytes());
        wav[28..32].copy_from_slice(&64000u32.to_le_bytes());
        wav[32..34].copy_from_slice(&4u16.to_le_bytes());
        assert_eq!(decode(&wav).unwrap().samples, vec![2000, -2000]);
        wav[40..44].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(decode(&wav).is_err());
    }
}
