/// Sound extraction from SSF2 .ssf files.
///
/// SSF2 stores sounds as DefineSound tags (SWF format code 6 = Nellymoser).
/// Fraymakers only supports WAV audio (with content-id based names); we emit
/// `library/audio/<name>.wav` + `.wav.meta`.
///
/// Pipeline:
///   SWF DefineSound bytes → minimal FLV container → ffmpeg → WAV
///
/// FLV is used as an intermediate because ffmpeg can't decode raw Nellymoser;
/// it needs a container that carries the codec+rate metadata.

use anyhow::{Result, bail, Context};
use std::collections::BTreeMap;
use std::path::Path;
use std::process::Command;

// ─── Sound format constants ───────────────────────────────────────────────────

const FMT_UNCOMPRESSED: u8 = 0;  // raw PCM, native (big) endian
const FMT_ADPCM:       u8 = 1;
const FMT_MP3:         u8 = 2;
const FMT_UNCOMPRESSED_LE: u8 = 3;  // raw PCM, little endian (WAV-native)
const FMT_NELLYMOSER8: u8 = 5;  // 8kHz mono only
const FMT_NELLYMOSER:  u8 = 6;  // variable rate

// Nellymoser frame = 64 bytes, decodes to 256 samples regardless of sample rate
const NELLY_FRAME_BYTES:   usize = 64;
const NELLY_SAMPLES_FRAME: u32   = 256;

// ─── Parsed sound ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct SoundEntry {
    /// SWF character id (matches SymbolClass mapping)
    pub char_id:      u16,
    /// Human-readable name from SymbolClass (e.g. "mario_jumpsfx")
    pub name:         String,
    /// SWF format code
    pub fmt:          u8,
    /// Sample rate in Hz (5512, 11025, 22050, 44100)
    pub sample_rate:  u32,
    /// True for 16-bit samples, false for 8-bit (uncompressed formats only)
    pub bits16:       bool,
    /// True for stereo, false for mono
    pub stereo:       bool,
    /// Total sample count
    pub sample_count: u32,
    /// Raw codec bytes (no SWF header)
    pub data:         Vec<u8>,
}

impl SoundEntry {
    pub fn duration_secs(&self) -> f32 {
        if self.sample_rate == 0 { return 0.0; }
        self.sample_count as f32 / self.sample_rate as f32
    }
}

// ─── Parse all DefineSound tags from an already-parsed SWF ───────────────────

pub fn parse_sounds(swf: &swf::Swf) -> Vec<SoundEntry> {
    let mut sounds:  BTreeMap<u16, SoundEntry> = BTreeMap::new();
    let mut symbols: BTreeMap<u16, String>     = BTreeMap::new();

    for tag in &swf.tags {
        match tag {
            swf::Tag::DefineSound(sound) => {
                // `AudioCompression as u8` is the raw SWF format nibble
                // (Uncompressed=3, Adpcm=1, Mp3=2, Nellymoser=6, …), which is
                // exactly what `convert_to_wav` matches on below. The crate has
                // already decoded `sample_rate` to Hz.
                sounds.insert(sound.id, SoundEntry {
                    char_id:      sound.id,
                    name:         String::new(),
                    fmt:          sound.format.compression as u8,
                    sample_rate:  sound.format.sample_rate as u32,
                    bits16:       sound.format.is_16_bit,
                    stereo:       sound.format.is_stereo,
                    sample_count: sound.num_samples,
                    data:         sound.data.to_vec(),
                });
            }
            // SymbolClass — maps char ids to AS3 class names
            swf::Tag::SymbolClass(links) => {
                for link in links {
                    symbols.insert(
                        link.id,
                        String::from_utf8_lossy(link.class_name.as_bytes()).to_string(),
                    );
                }
            }
            _ => {}
        }
    }

    // Attach names from SymbolClass
    for (id, entry) in &mut sounds {
        if let Some(name) = symbols.get(id) {
            entry.name = name.clone();
        }
        if entry.name.is_empty() {
            entry.name = format!("sound_{}", id);
        }
    }

    sounds.into_values().collect()
}

// ─── Convert one SoundEntry to WAV via FLV intermediary ──────────────────────

pub fn convert_to_wav(entry: &SoundEntry, out_path: &Path) -> Result<()> {
    match entry.fmt {
        FMT_NELLYMOSER | FMT_NELLYMOSER8 => convert_nellymoser_to_wav(entry, out_path),
        FMT_MP3   => convert_mp3_to_wav(entry, out_path),
        FMT_ADPCM => convert_via_flv(entry, out_path),
        FMT_UNCOMPRESSED | FMT_UNCOMPRESSED_LE => convert_pcm_to_wav(entry, out_path),
        other => bail!("Unsupported sound format {} for '{}'", other, entry.name),
    }
}

/// Write raw uncompressed PCM (SWF format 0 = big-endian, format 3 = little-endian)
/// directly to a WAV file. WAV is little-endian PCM, so format 3 copies verbatim;
/// format 0 16-bit samples are byte-swapped to LE.
fn convert_pcm_to_wav(entry: &SoundEntry, out_path: &Path) -> Result<()> {
    let channels: u16 = if entry.stereo { 2 } else { 1 };
    let bits_per_sample: u16 = if entry.bits16 { 16 } else { 8 };

    // Normalise sample bytes to little-endian for WAV.
    let mut pcm = entry.data.clone();
    if entry.fmt == FMT_UNCOMPRESSED && entry.bits16 {
        // Native-endian Flash PCM is big-endian; swap 16-bit sample byte pairs.
        for pair in pcm.chunks_exact_mut(2) {
            pair.swap(0, 1);
        }
    }

    let byte_rate = entry.sample_rate * channels as u32 * bits_per_sample as u32 / 8;
    let block_align = channels * bits_per_sample / 8;
    let data_len = pcm.len() as u32;

    let mut wav = Vec::with_capacity(44 + pcm.len());
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36 + data_len).to_le_bytes());
    wav.extend_from_slice(b"WAVE");
    wav.extend_from_slice(b"fmt ");
    wav.extend_from_slice(&16u32.to_le_bytes());      // PCM fmt chunk size
    wav.extend_from_slice(&1u16.to_le_bytes());        // audioFormat = PCM
    wav.extend_from_slice(&channels.to_le_bytes());
    wav.extend_from_slice(&entry.sample_rate.to_le_bytes());
    wav.extend_from_slice(&byte_rate.to_le_bytes());
    wav.extend_from_slice(&block_align.to_le_bytes());
    wav.extend_from_slice(&bits_per_sample.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&data_len.to_le_bytes());
    wav.extend_from_slice(&pcm);

    std::fs::write(out_path, &wav)
        .with_context(|| format!("write PCM WAV '{}'", entry.name))?;
    Ok(())
}

fn convert_nellymoser_to_wav(entry: &SoundEntry, out_path: &Path) -> Result<()> {
    // Wrap raw Nellymoser bytes in a minimal FLV audio-only file, then ffmpeg → WAV
    let flv = build_nellymoser_flv(entry);

    let tmp_flv = out_path.with_extension("tmp.flv");
    std::fs::write(&tmp_flv, &flv)?;

    let output = Command::new("ffmpeg")
        .args(["-y", "-i"])
        .arg(&tmp_flv)
        .args(["-c:a", "pcm_s16le", "-vn"])
        .arg(out_path)
        .output()?;

    let _ = std::fs::remove_file(&tmp_flv);

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let msg: String = stderr.lines().rev().take(3).collect::<Vec<_>>().into_iter().rev().collect::<Vec<_>>().join(" | ");
        bail!("ffmpeg failed for '{}': {}", entry.name, msg);
    }
    Ok(())
}

fn convert_mp3_to_wav(entry: &SoundEntry, out_path: &Path) -> Result<()> {
    // SWF MP3 has a 2-byte SeekSamples header we skip
    let mp3_data = if entry.data.len() > 2 { &entry.data[2..] } else { &entry.data[..] };
    let tmp_mp3 = out_path.with_extension("tmp.mp3");
    std::fs::write(&tmp_mp3, mp3_data)?;

    let status = Command::new("ffmpeg")
        .args(["-y", "-i"])
        .arg(&tmp_mp3)
        .args(["-c:a", "pcm_s16le", "-vn"])
        .arg(out_path)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()?;

    let _ = std::fs::remove_file(&tmp_mp3);
    if !status.success() {
        bail!("ffmpeg failed converting MP3 '{}' to WAV", entry.name);
    }
    Ok(())
}

fn convert_via_flv(entry: &SoundEntry, out_path: &Path) -> Result<()> {
    // Generic FLV wrapper for ADPCM
    let flv = build_generic_flv(entry);
    let tmp = out_path.with_extension("tmp.flv");
    std::fs::write(&tmp, &flv)?;
    let status = Command::new("ffmpeg")
        .args(["-y", "-i"]).arg(&tmp)
        .args(["-c:a", "pcm_s16le", "-vn"])
        .arg(out_path)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()?;
    let _ = std::fs::remove_file(&tmp);
    if !status.success() {
        bail!("ffmpeg failed converting ADPCM '{}' to WAV", entry.name);
    }
    Ok(())
}

// ─── FLV builder ──────────────────────────────────────────────────────────────

fn build_nellymoser_flv(entry: &SoundEntry) -> Vec<u8> {
    // FLV audio header byte: (fmt<<4)|(rate_idx<<2)|(bits16<<1)|stereo
    // Nellymoser is always mono, but we use the rate from the SWF flags
    let rate_idx = match entry.sample_rate {
        5512  => 0u8,
        11025 => 1,
        22050 => 2,
        _     => 3,  // 44100
    };
    let fmt_nibble = if entry.fmt == FMT_NELLYMOSER8 { 5u8 } else { 6u8 };
    let audio_hdr = (fmt_nibble << 4) | (rate_idx << 2) | (1 << 1);  // 16-bit, mono

    build_flv_from_chunks(audio_hdr, &entry.data, NELLY_FRAME_BYTES, NELLY_SAMPLES_FRAME, entry.sample_rate)
}

fn build_generic_flv(entry: &SoundEntry) -> Vec<u8> {
    let rate_idx = match entry.sample_rate {
        5512  => 0u8,
        11025 => 1,
        22050 => 2,
        _     => 3,
    };
    let audio_hdr = (entry.fmt << 4) | (rate_idx << 2) | (1 << 1);
    // For ADPCM: first chunk has an extra header byte; just send as one big tag
    build_flv_from_chunks(audio_hdr, &entry.data, entry.data.len().max(1), entry.sample_count, entry.sample_rate)
}

fn build_flv_from_chunks(audio_hdr: u8, data: &[u8], chunk_frames: usize, samples_per_frame: u32, sample_rate: u32) -> Vec<u8> {
    // FLV file header: "FLV" + version(1) + flags(has_audio=4) + header_size(9) + prev_tag_size=0
    let mut flv = b"FLV\x01\x04\x00\x00\x00\x09\x00\x00\x00\x00".to_vec();

    let mut offset = 0usize;
    let mut samples_so_far = 0u32;

    while offset < data.len() {
        let end = (offset + chunk_frames).min(data.len());
        let chunk = &data[offset..end];

        let ts_ms = if sample_rate > 0 {
            (samples_so_far as u64 * 1000 / sample_rate as u64) as u32
        } else { 0 };

        // Audio tag payload = audio_hdr + chunk
        let payload: Vec<u8> = std::iter::once(audio_hdr).chain(chunk.iter().copied()).collect();
        let size = payload.len() as u32;

        // FLV tag: type(1) + data_size(3) + timestamp(3) + ts_extended(1) + stream_id(3) + payload
        flv.push(8); // audio tag type
        flv.extend_from_slice(&size.to_be_bytes()[1..]);           // 3 bytes
        flv.extend_from_slice(&(ts_ms & 0xFFFFFF).to_be_bytes()[1..]); // 3 bytes
        flv.push((ts_ms >> 24) as u8);                             // ts extended
        flv.extend_from_slice(&[0u8, 0, 0]);                       // stream id
        flv.extend_from_slice(&payload);

        // Previous tag size = 11 (tag header) + payload size
        let prev_size = (11 + size).to_be_bytes();
        flv.extend_from_slice(&prev_size);

        let n_frames = (end - offset) / chunk_frames.max(1);
        samples_so_far += n_frames as u32 * samples_per_frame;
        offset = end;
    }

    flv
}

// ─── Bulk extract all sounds from a character ────────────────────────────────

pub fn extract_all_sounds(swf: &swf::Swf, out_dir: &Path, char_id: &str) -> Result<Vec<SoundEntry>> {
    let sounds = parse_sounds(swf);
    if sounds.is_empty() {
        log::info!("No sounds found in SWF");
        return Ok(sounds);
    }

    std::fs::create_dir_all(out_dir)?;

    // Check ffmpeg is available
    if Command::new("ffmpeg").arg("-version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status().is_err()
    {
        log::warn!("ffmpeg not found — skipping sound extraction. Install ffmpeg to extract audio.");
        return Ok(sounds);
    }

    let mut ok = 0usize;
    let mut skip = 0usize;

    for entry in &sounds {
        // Sanitize name for filesystem
        let safe_name: String = entry.name.chars()
            .map(|c| if c.is_alphanumeric() || c == '_' || c == '-' { c } else { '_' })
            .collect();

        let out_path = out_dir.join(format!("{}.wav", safe_name));

        if out_path.exists() {
            skip += 1;
            continue;
        }

        match convert_to_wav(entry, &out_path) {
            Ok(()) => {
                log::debug!("  {} → {}.wav ({:.2}s)", entry.name, safe_name, entry.duration_secs());
                ok += 1;
            }
            Err(e) => {
                log::warn!("  sound '{}' conversion failed: {}", entry.name, e);
            }
        }
    }

    log::info!("sound_extractor: {} converted, {} skipped ({}→{} total)",
        ok, skip, sounds.len(), char_id);

    Ok(sounds)
}
