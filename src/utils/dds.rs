use std::io::{Cursor, Error, ErrorKind, Read, Result, Write};

use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};

pub const DDS_MAGIC: u32 = 0x20534444;

const DDSD_CAPS: u32 = 0x1;
const DDSD_HEIGHT: u32 = 0x2;
const DDSD_WIDTH: u32 = 0x4;
const DDSD_PIXELFORMAT: u32 = 0x1000;
const DDSD_MIPMAPCOUNT: u32 = 0x20000;
const DDSD_LINEARSIZE: u32 = 0x80000;
const DDSD_PITCH: u32 = 0x8;

const DDPF_ALPHAPIXELS: u32 = 0x1;
const DDPF_FOURCC: u32 = 0x4;
const DDPF_RGB: u32 = 0x40;
const DDPF_LUMINANCE: u32 = 0x20000;

const DDSCAPS_COMPLEX: u32 = 0x8;
const DDSCAPS_TEXTURE: u32 = 0x1000;
const DDSCAPS_MIPMAP: u32 = 0x400000;

const DDS_DIMENSION_TEXTURE2D: u32 = 3;

const fn fourcc(b: &[u8; 4]) -> u32 {
    u32::from_le_bytes(*b)
}
const FCC_DXT1: u32 = fourcc(b"DXT1");
const FCC_DXT3: u32 = fourcc(b"DXT3");
const FCC_DXT5: u32 = fourcc(b"DXT5");
const FCC_ATI2: u32 = fourcc(b"ATI2");
const FCC_DX10: u32 = fourcc(b"DX10");

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
#[repr(u32)]
pub enum DxgiFormat {
    R16G16B16A16Float = 10,
    R32G32B32A32Float = 2,
    BC5Unorm = 83,
}

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub enum PixelFormat {
    Dxt1,
    Dxt3,
    Dxt5,
    A8R8G8B8,
    G8,
    Bc5,
    FloatRgba,
}

impl PixelFormat {
    pub fn from_pf_label(s: &str) -> Option<Self> {
        let tail = s.rsplit("::").next().unwrap_or(s);
        Some(match tail {
            "PF_DXT1" => Self::Dxt1,
            "PF_DXT3" => Self::Dxt3,
            "PF_DXT5" => Self::Dxt5,
            "PF_A8R8G8B8" => Self::A8R8G8B8,
            "PF_G8" => Self::G8,
            "PF_BC5" => Self::Bc5,
            "PF_FloatRGBA" => Self::FloatRgba,
            _ => return None,
        })
    }

    pub fn as_pf_label(&self) -> &'static str {
        match self {
            Self::Dxt1 => "PF_DXT1",
            Self::Dxt3 => "PF_DXT3",
            Self::Dxt5 => "PF_DXT5",
            Self::A8R8G8B8 => "PF_A8R8G8B8",
            Self::G8 => "PF_G8",
            Self::Bc5 => "PF_BC5",
            Self::FloatRgba => "PF_FloatRGBA",
        }
    }

    pub fn is_block_compressed(&self) -> bool {
        matches!(self, Self::Dxt1 | Self::Dxt3 | Self::Dxt5 | Self::Bc5)
    }

    pub fn unit_bytes(&self) -> u32 {
        match self {
            Self::Dxt1 => 8,
            Self::Dxt3 | Self::Dxt5 | Self::Bc5 => 16,
            Self::A8R8G8B8 => 4,
            Self::G8 => 1,
            Self::FloatRgba => 8,
        }
    }

    pub fn mip_size(&self, w: u32, h: u32) -> u32 {
        if self.is_block_compressed() {
            let bw = w.max(4).div_ceil(4);
            let bh = h.max(4).div_ceil(4);
            bw * bh * self.unit_bytes()
        } else {
            w * h * self.unit_bytes()
        }
    }
}

#[derive(Debug, Clone)]
pub struct DdsMip {
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct Dds {
    pub format: PixelFormat,
    pub mips: Vec<DdsMip>,
}

impl Dds {
    pub fn encode(&self) -> Result<Vec<u8>> {
        let mut out =
            Vec::with_capacity(128 + 20 + self.mips.iter().map(|m| m.data.len()).sum::<usize>());
        let mut w = Cursor::new(&mut out);

        if self.mips.is_empty() {
            return Err(Error::new(
                ErrorKind::InvalidInput,
                "DDS must have at least one mip",
            ));
        }
        let top = &self.mips[0];

        w.write_u32::<LittleEndian>(DDS_MAGIC)?;

        let mut flags = DDSD_CAPS | DDSD_HEIGHT | DDSD_WIDTH | DDSD_PIXELFORMAT;
        let mut caps = DDSCAPS_TEXTURE;
        let mip_count = self.mips.len() as u32;
        if mip_count > 1 {
            flags |= DDSD_MIPMAPCOUNT;
            caps |= DDSCAPS_MIPMAP | DDSCAPS_COMPLEX;
        }

        let pitch_or_lsize = if self.format.is_block_compressed() {
            flags |= DDSD_LINEARSIZE;
            self.format.mip_size(top.width, top.height)
        } else {
            flags |= DDSD_PITCH;
            (top.width * self.format.unit_bytes() * 8 + 7) / 8
        };

        w.write_u32::<LittleEndian>(124)?;
        w.write_u32::<LittleEndian>(flags)?;
        w.write_u32::<LittleEndian>(top.height)?;
        w.write_u32::<LittleEndian>(top.width)?;
        w.write_u32::<LittleEndian>(pitch_or_lsize)?;
        w.write_u32::<LittleEndian>(0)?;
        w.write_u32::<LittleEndian>(mip_count)?;
        for _ in 0..11 {
            w.write_u32::<LittleEndian>(0)?;
        }

        write_pixelformat(&mut w, self.format)?;

        w.write_u32::<LittleEndian>(caps)?;
        w.write_u32::<LittleEndian>(0)?;
        w.write_u32::<LittleEndian>(0)?;
        w.write_u32::<LittleEndian>(0)?;
        w.write_u32::<LittleEndian>(0)?;

        if let Some(dxgi) = needs_dx10(self.format) {
            w.write_u32::<LittleEndian>(dxgi as u32)?;
            w.write_u32::<LittleEndian>(DDS_DIMENSION_TEXTURE2D)?;
            w.write_u32::<LittleEndian>(0)?;
            w.write_u32::<LittleEndian>(1)?;
            w.write_u32::<LittleEndian>(0)?;
        }

        for m in &self.mips {
            w.write_all(&m.data)?;
        }
        Ok(out)
    }

    pub fn decode(bytes: &[u8]) -> Result<Dds> {
        let mut c = Cursor::new(bytes);
        let magic = c.read_u32::<LittleEndian>()?;
        if magic != DDS_MAGIC {
            return Err(Error::new(
                ErrorKind::InvalidData,
                format!("not a DDS file: magic=0x{magic:08x}"),
            ));
        }
        let size = c.read_u32::<LittleEndian>()?;
        if size != 124 {
            return Err(Error::new(
                ErrorKind::InvalidData,
                format!("bad DDS_HEADER size {size}"),
            ));
        }
        let _flags = c.read_u32::<LittleEndian>()?;
        let height = c.read_u32::<LittleEndian>()?;
        let width = c.read_u32::<LittleEndian>()?;
        let _pls = c.read_u32::<LittleEndian>()?;
        let _depth = c.read_u32::<LittleEndian>()?;
        let mut mip_count = c.read_u32::<LittleEndian>()?;
        for _ in 0..11 {
            c.read_u32::<LittleEndian>()?;
        }

        let pf_size = c.read_u32::<LittleEndian>()?;
        if pf_size != 32 {
            return Err(Error::new(
                ErrorKind::InvalidData,
                format!("bad DDS_PIXELFORMAT size {pf_size}"),
            ));
        }
        let pf_flags = c.read_u32::<LittleEndian>()?;
        let fcc = c.read_u32::<LittleEndian>()?;
        let bit_count = c.read_u32::<LittleEndian>()?;
        let r_mask = c.read_u32::<LittleEndian>()?;
        let _g_mask = c.read_u32::<LittleEndian>()?;
        let _b_mask = c.read_u32::<LittleEndian>()?;
        let a_mask = c.read_u32::<LittleEndian>()?;
        let _caps = c.read_u32::<LittleEndian>()?;
        let _caps2 = c.read_u32::<LittleEndian>()?;
        let _caps3 = c.read_u32::<LittleEndian>()?;
        let _caps4 = c.read_u32::<LittleEndian>()?;
        let _reserved = c.read_u32::<LittleEndian>()?;

        let format = if pf_flags & DDPF_FOURCC != 0 {
            if fcc == FCC_DX10 {
                let dxgi = c.read_u32::<LittleEndian>()?;
                let _rd = c.read_u32::<LittleEndian>()?;
                let _mf = c.read_u32::<LittleEndian>()?;
                let _as_ = c.read_u32::<LittleEndian>()?;
                let _mf2 = c.read_u32::<LittleEndian>()?;
                match dxgi {
                    x if x == DxgiFormat::BC5Unorm as u32 => PixelFormat::Bc5,
                    x if x == DxgiFormat::R16G16B16A16Float as u32 => PixelFormat::FloatRgba,
                    x => {
                        return Err(Error::new(
                            ErrorKind::Unsupported,
                            format!("unsupported DXGI format {x}"),
                        ));
                    }
                }
            } else {
                match fcc {
                    FCC_DXT1 => PixelFormat::Dxt1,
                    FCC_DXT3 => PixelFormat::Dxt3,
                    FCC_DXT5 => PixelFormat::Dxt5,
                    FCC_ATI2 => PixelFormat::Bc5,
                    other => {
                        return Err(Error::new(
                            ErrorKind::Unsupported,
                            format!("unsupported DDS FourCC 0x{other:08x}"),
                        ));
                    }
                }
            }
        } else if pf_flags & DDPF_RGB != 0
            && bit_count == 32
            && (pf_flags & DDPF_ALPHAPIXELS != 0 || a_mask != 0)
        {
            PixelFormat::A8R8G8B8
        } else if pf_flags & DDPF_LUMINANCE != 0 && bit_count == 8 && r_mask == 0xff {
            PixelFormat::G8
        } else {
            return Err(Error::new(
                ErrorKind::Unsupported,
                format!("unsupported DDS pixel format (flags=0x{pf_flags:x}, bpp={bit_count})"),
            ));
        };

        if mip_count == 0 {
            mip_count = 1;
        }

        let mut mips = Vec::with_capacity(mip_count as usize);
        let mut w = width;
        let mut h = height;
        for _ in 0..mip_count {
            let n = format.mip_size(w, h) as usize;
            let mut buf = vec![0u8; n];
            c.read_exact(&mut buf)?;
            mips.push(DdsMip {
                width: w,
                height: h,
                data: buf,
            });
            w = (w / 2).max(1);
            h = (h / 2).max(1);
        }

        Ok(Dds { format, mips })
    }
}

fn needs_dx10(p: PixelFormat) -> Option<DxgiFormat> {
    Some(match p {
        PixelFormat::Bc5 => DxgiFormat::BC5Unorm,
        PixelFormat::FloatRgba => DxgiFormat::R16G16B16A16Float,
        _ => return None,
    })
}

fn write_pixelformat(w: &mut Cursor<&mut Vec<u8>>, p: PixelFormat) -> Result<()> {
    w.write_u32::<LittleEndian>(32)?;
    match p {
        PixelFormat::Dxt1 | PixelFormat::Dxt3 | PixelFormat::Dxt5 => {
            let fcc = match p {
                PixelFormat::Dxt1 => FCC_DXT1,
                PixelFormat::Dxt3 => FCC_DXT3,
                PixelFormat::Dxt5 => FCC_DXT5,
                _ => unreachable!(),
            };
            w.write_u32::<LittleEndian>(DDPF_FOURCC)?;
            w.write_u32::<LittleEndian>(fcc)?;
            for _ in 0..5 {
                w.write_u32::<LittleEndian>(0)?;
            }
        }
        PixelFormat::Bc5 | PixelFormat::FloatRgba => {
            w.write_u32::<LittleEndian>(DDPF_FOURCC)?;
            w.write_u32::<LittleEndian>(FCC_DX10)?;
            for _ in 0..5 {
                w.write_u32::<LittleEndian>(0)?;
            }
        }
        PixelFormat::A8R8G8B8 => {
            w.write_u32::<LittleEndian>(DDPF_RGB | DDPF_ALPHAPIXELS)?;
            w.write_u32::<LittleEndian>(0)?;
            w.write_u32::<LittleEndian>(32)?;
            w.write_u32::<LittleEndian>(0x00ff0000)?;
            w.write_u32::<LittleEndian>(0x0000ff00)?;
            w.write_u32::<LittleEndian>(0x000000ff)?;
            w.write_u32::<LittleEndian>(0xff000000)?;
        }
        PixelFormat::G8 => {
            w.write_u32::<LittleEndian>(DDPF_LUMINANCE)?;
            w.write_u32::<LittleEndian>(0)?;
            w.write_u32::<LittleEndian>(8)?;
            w.write_u32::<LittleEndian>(0xff)?;
            w.write_u32::<LittleEndian>(0)?;
            w.write_u32::<LittleEndian>(0)?;
            w.write_u32::<LittleEndian>(0)?;
        }
    }
    Ok(())
}
