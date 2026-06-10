use std::{
    fs::File,
    io::{Cursor, Result, Write},
    path::{Path, PathBuf},
};

use byteorder::{LittleEndian, WriteBytesExt};

use crate::{
    native::{BulkBlock, NativePayload, NativeRead, NativeReadCtx, NativeSerializer},
    upkprops::{Property, PropertyValue},
};

#[derive(Debug, Clone)]
pub struct SoundNodeWavePayload {
    pub raw_data: BulkBlock,
    pub compressed_pc: BulkBlock,
    pub compressed_xbox360: BulkBlock,
    pub compressed_ps3: BulkBlock,
    pub trailing_raw: Vec<u8>,
    pub num_channels: Option<i32>,
    pub sample_rate: Option<i32>,
    pub duration: Option<f32>,
    pub channel_offsets: Vec<i32>,
    pub channel_sizes: Vec<i32>,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum AudioSniff {
    Empty,
    OggVorbis,
    RiffWave,
    Mp3,
    Xma,
    Unknown,
}

impl AudioSniff {
    pub fn of(bytes: &[u8]) -> Self {
        if bytes.is_empty() {
            return Self::Empty;
        }
        if bytes.starts_with(b"OggS") {
            return Self::OggVorbis;
        }
        if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WAVE" {
            return if bytes
                .windows(4)
                .take(64)
                .any(|w| w == b"XMA2" || w == b"XMA ")
            {
                Self::Xma
            } else {
                Self::RiffWave
            };
        }
        if bytes.starts_with(b"ID3")
            || (bytes.len() >= 2 && bytes[0] == 0xff && (bytes[1] & 0xe0) == 0xe0)
        {
            return Self::Mp3;
        }
        Self::Unknown
    }

    pub fn extension(self) -> &'static str {
        match self {
            Self::OggVorbis => "ogg",
            Self::RiffWave => "wav",
            Self::Mp3 => "mp3",
            Self::Xma => "xma",
            Self::Empty | Self::Unknown => "bin",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::OggVorbis => "ogg-vorbis",
            Self::RiffWave => "riff-wave",
            Self::Mp3 => "mp3",
            Self::Xma => "xma",
            Self::Empty => "empty",
            Self::Unknown => "unknown",
        }
    }
}

fn prop_int(props: &[Property], name: &str) -> Option<i32> {
    props
        .iter()
        .find(|p| p.name == name)
        .and_then(|p| match &p.value {
            PropertyValue::Int(i) => Some(*i),
            _ => None,
        })
}

fn prop_float(props: &[Property], name: &str) -> Option<f32> {
    props
        .iter()
        .find(|p| p.name == name)
        .and_then(|p| match &p.value {
            PropertyValue::Float(f) => Some(*f),
            _ => None,
        })
}

fn prop_int_array(props: &[Property], name: &str) -> Vec<i32> {
    let Some(p) = props.iter().find(|p| p.name == name) else {
        return Vec::new();
    };
    match &p.value {
        PropertyValue::Array(items) => items
            .iter()
            .filter_map(|v| match v {
                PropertyValue::Int(i) => Some(*i),
                _ => None,
            })
            .collect(),
        PropertyValue::Raw(buf) => {
            let mut out = Vec::with_capacity(buf.len() / 4);
            for chunk in buf.chunks_exact(4) {
                out.push(i32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]));
            }
            out
        }
        _ => Vec::new(),
    }
}

fn wrap_pcm_as_wav(
    pcm: &[u8],
    sample_rate: u32,
    num_channels: u16,
    bits_per_sample: u16,
) -> Result<Vec<u8>> {
    let byte_rate = sample_rate * num_channels as u32 * bits_per_sample as u32 / 8;
    let block_align = num_channels * bits_per_sample / 8;
    let data_size = pcm.len() as u32;
    let riff_size = 4 + (8 + 16) + (8 + data_size);

    let mut out = Vec::with_capacity(8 + riff_size as usize);
    out.write_all(b"RIFF")?;
    out.write_u32::<LittleEndian>(riff_size)?;
    out.write_all(b"WAVE")?;

    out.write_all(b"fmt ")?;
    out.write_u32::<LittleEndian>(16)?;
    out.write_u16::<LittleEndian>(1)?; // PCM
    out.write_u16::<LittleEndian>(num_channels)?;
    out.write_u32::<LittleEndian>(sample_rate)?;
    out.write_u32::<LittleEndian>(byte_rate)?;
    out.write_u16::<LittleEndian>(block_align)?;
    out.write_u16::<LittleEndian>(bits_per_sample)?;

    out.write_all(b"data")?;
    out.write_u32::<LittleEndian>(data_size)?;
    out.write_all(pcm)?;
    Ok(out)
}

pub struct SoundNodeWaveSer;

impl NativeSerializer for SoundNodeWaveSer {
    fn class_name(&self) -> &'static str {
        "SoundNodeWave"
    }

    fn read(&self, ctx: &NativeReadCtx) -> Result<NativeRead> {
        let mut c = Cursor::new(ctx.blob);

        let raw_data = BulkBlock::read(&mut c)?;
        let compressed_pc = BulkBlock::read(&mut c)?;
        let compressed_xbox360 = BulkBlock::read(&mut c)?;
        let compressed_ps3 = BulkBlock::read(&mut c)?;

        let pos = c.position() as usize;
        let trailing_raw = if pos < ctx.blob.len() {
            ctx.blob[pos..].to_vec()
        } else {
            Vec::new()
        };
        if !trailing_raw.is_empty() {
            eprintln!(
                "  \x1b[33msnd\x1b[0m: {} trailing bytes after 4 bulk blocks (ver={}); preserved as raw",
                trailing_raw.len(),
                ctx.ver
            );
        }

        let payload = SoundNodeWavePayload {
            raw_data,
            compressed_pc,
            compressed_xbox360,
            compressed_ps3,
            trailing_raw,
            num_channels: prop_int(ctx.props, "NumChannels"),
            sample_rate: prop_int(ctx.props, "SampleRate"),
            duration: prop_float(ctx.props, "Duration"),
            channel_offsets: prop_int_array(ctx.props, "ChannelOffsets"),
            channel_sizes: prop_int_array(ctx.props, "ChannelSizes"),
        };

        Ok(NativeRead::just(NativePayload::SoundNodeWave(payload)))
    }

    fn emit_external(
        &self,
        payload: &NativePayload,
        dir: &Path,
        stem: &str,
    ) -> Result<Vec<PathBuf>> {
        let p = match payload {
            NativePayload::SoundNodeWave(p) => p,
            _ => return Ok(Vec::new()),
        };

        let mut out = Vec::new();

        if !p.compressed_pc.data.is_empty() {
            let sniff = AudioSniff::of(&p.compressed_pc.data);
            let ext = sniff.extension();
            let path = dir.join(format!("{stem}.{ext}"));
            File::create(&path)?.write_all(&p.compressed_pc.data)?;
            println!(
                "  \x1b[36msnd\x1b[0m → \x1b[32m{}\x1b[0m  ({} bytes, {})",
                path.display(),
                p.compressed_pc.data.len(),
                sniff.label()
            );
            out.push(path);
        }

        if !p.raw_data.data.is_empty() {
            let sniff = AudioSniff::of(&p.raw_data.data);
            let bytes = match sniff {
                AudioSniff::RiffWave => p.raw_data.data.clone(),
                _ => {
                    let sr = p.sample_rate.unwrap_or(0).max(0) as u32;
                    let ch = p.num_channels.unwrap_or(1).max(1) as u16;
                    if sr == 0 {
                        eprintln!(
                            "  \x1b[33msnd\x1b[0m: {stem} RawData present but SampleRate=0; \
                             writing .pcm with no header"
                        );
                        let path = dir.join(format!("{stem}.raw.pcm"));
                        File::create(&path)?.write_all(&p.raw_data.data)?;
                        out.push(path);
                        return Ok(out);
                    }
                    wrap_pcm_as_wav(&p.raw_data.data, sr, ch, 16)?
                }
            };
            let path = dir.join(format!("{stem}.raw.wav"));
            File::create(&path)?.write_all(&bytes)?;
            println!(
                "  \x1b[36msnd\x1b[0m → \x1b[32m{}\x1b[0m  ({} bytes raw PCM)",
                path.display(),
                p.raw_data.data.len()
            );
            out.push(path);
        }

        for (block, suffix) in [
            (&p.compressed_xbox360, "xbox360"),
            (&p.compressed_ps3, "ps3"),
        ] {
            if block.data.is_empty() {
                continue;
            }
            let sniff = AudioSniff::of(&block.data);
            let ext = sniff.extension();
            let path = dir.join(format!("{stem}.{suffix}.{ext}"));
            File::create(&path)?.write_all(&block.data)?;
            println!(
                "  \x1b[36msnd\x1b[0m → \x1b[32m{}\x1b[0m  ({} bytes, {})",
                path.display(),
                block.data.len(),
                sniff.label()
            );
            out.push(path);
        }

        Ok(out)
    }
}
