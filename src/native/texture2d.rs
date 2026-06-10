use std::{
    fs::File,
    io::{Cursor, Error, ErrorKind, Read, Result, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
};

use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};
use serde::{Deserialize, Serialize};

use crate::{
    native::{NativePayload, NativeRead, NativeReadCtx, NativeSerializer},
    schemadb::SchemaDb,
    upkprops::{Property, PropertyValue},
    utils::dds::{Dds, DdsMip, PixelFormat},
    versions::{
        BULKDATA_SERIALIZE_COMPRESSED, BULKDATA_STORE_IN_SEPARATE_FILE,
        VER_ADDED_CACHED_IPHONE_DATA, VER_ADDED_TEXTURE_FILECACHE_GUIDS, VER_ANDROID_ETC_SEPARATED,
        VER_VERSION_NUMBER_FIX_FOR_FLASH_TEXTURES,
    },
};

use super::BulkBlock;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Mip {
    pub flags: u32,
    pub element_count: i32,
    pub size_on_disk: i32,
    pub offset_in_file: i32,
    pub size_x: i32,
    pub size_y: i32,
    pub source: MipSource,
    #[serde(skip)]
    pub data: Vec<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Eq, PartialEq)]
pub enum MipSource {
    Inline,
    Tfc { stem_lc: String },
    Missing,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Texture2DPayload {
    pub mips: Vec<Mip>,
    pub tfc_guid: [i32; 4],
    pub cached_pvrtc_mips: Vec<Mip>,
    pub trailing_raw: Vec<u8>,

    pub format_label: Option<String>,
    pub tfc_name: Option<String>,
}

fn read_bulk_data<R: Read + Seek>(r: &mut R) -> Result<(u32, i32, i32, i32, Vec<u8>)> {
    let flags = r.read_u32::<LittleEndian>()?;
    let element_count = r.read_i32::<LittleEndian>()?;
    let size_on_disk = r.read_i32::<LittleEndian>()?;
    let offset_in_file = r.read_i32::<LittleEndian>()?;

    let inline = flags & BULKDATA_STORE_IN_SEPARATE_FILE == 0;
    let data = if inline && size_on_disk > 0 {
        let mut buf = vec![0u8; size_on_disk as usize];
        r.read_exact(&mut buf)?;
        buf
    } else {
        Vec::new()
    };
    Ok((flags, element_count, size_on_disk, offset_in_file, data))
}

fn read_indirect_mips<R: Read + Seek>(r: &mut R) -> Result<Vec<Mip>> {
    let count = r.read_i32::<LittleEndian>()?;
    if !(0..=64).contains(&count) {
        return Err(Error::new(
            ErrorKind::InvalidData,
            format!("implausible mip count {count}"),
        ));
    }
    let mut mips = Vec::with_capacity(count as usize);
    for _ in 0..count {
        let (flags, element_count, size_on_disk, offset_in_file, data) = read_bulk_data(r)?;
        let size_x = r.read_i32::<LittleEndian>()?;
        let size_y = r.read_i32::<LittleEndian>()?;
        let source = if !data.is_empty() {
            MipSource::Inline
        } else if flags & BULKDATA_STORE_IN_SEPARATE_FILE != 0 && size_on_disk > 0 {
            MipSource::Tfc {
                stem_lc: String::new(),
            }
        } else {
            MipSource::Missing
        };
        mips.push(Mip {
            flags,
            element_count,
            size_on_disk,
            offset_in_file,
            size_x,
            size_y,
            source,
            data,
        });
    }
    Ok(mips)
}

fn skip_byte_bulk_data<R: Read + Seek>(r: &mut R) -> Result<()> {
    let flags = r.read_u32::<LittleEndian>()?;
    let _ec = r.read_i32::<LittleEndian>()?;
    let sod = r.read_i32::<LittleEndian>()?;
    let _off = r.read_i32::<LittleEndian>()?;
    if flags & BULKDATA_STORE_IN_SEPARATE_FILE == 0 && sod > 0 {
        let mut sink = vec![0u8; sod as usize];
        r.read_exact(&mut sink)?;
    }
    Ok(())
}

impl Texture2DPayload {
    fn parse_bytes(tail: &[u8], ver: i16) -> Result<Self> {
        let mut c = Cursor::new(tail);
        let _source_art = BulkBlock::read(&mut c)?;
        let mips = read_indirect_mips(&mut c)?;

        let tfc_guid = if ver >= VER_ADDED_TEXTURE_FILECACHE_GUIDS {
            [
                c.read_i32::<LittleEndian>()?,
                c.read_i32::<LittleEndian>()?,
                c.read_i32::<LittleEndian>()?,
                c.read_i32::<LittleEndian>()?,
            ]
        } else {
            [0; 4]
        };

        let cached_pvrtc_mips = if ver >= VER_ADDED_CACHED_IPHONE_DATA {
            read_indirect_mips(&mut c)?
        } else {
            Vec::new()
        };

        let pos = c.position() as usize;
        let trailing_raw = if pos < tail.len() {
            tail[pos..].to_vec()
        } else {
            Vec::new()
        };

        if !trailing_raw.is_empty()
            && (ver >= VER_VERSION_NUMBER_FIX_FOR_FLASH_TEXTURES
                || ver >= VER_ANDROID_ETC_SEPARATED)
        {
            eprintln!(
                "  \x1b[33mtex\x1b[0m: {} trailing bytes after PVRTC mips (ver={}); preserved as raw",
                trailing_raw.len(),
                ver
            );
        }

        Ok(Self {
            mips,
            tfc_guid,
            cached_pvrtc_mips,
            trailing_raw,
            format_label: None,
            tfc_name: None,
        })
    }
}

fn prop_enum_label<'a>(props: &'a [Property], name: &str) -> Option<&'a str> {
    props
        .iter()
        .find(|p| p.name == name)
        .and_then(|p| match &p.value {
            PropertyValue::EnumLabel(s) => Some(s.as_str()),
            _ => None,
        })
}

fn prop_string_or_name<'a>(
    props: &'a [Property],
    name: &str,
    pak: &crate::upkreader::UPKPak,
) -> Option<String> {
    let p = props.iter().find(|p| p.name == name)?;
    match &p.value {
        PropertyValue::String(s) => Some(s.clone()),
        PropertyValue::Name(fn_) => Some(pak.fname_to_string(fn_)),
        _ => None,
    }
}

fn resolve_tfc_payload(mip: &Mip, tfc_stem: &str, db: &SchemaDb) -> Result<Option<Vec<u8>>> {
    if mip.flags & BULKDATA_STORE_IN_SEPARATE_FILE == 0 {
        return Ok(None);
    }
    if mip.flags & BULKDATA_SERIALIZE_COMPRESSED != 0 {
        eprintln!(
            "  \x1b[33mtfc\x1b[0m: compressed payload in '{tfc_stem}.tfc' \
             (flags=0x{:x}) — skipping inline extract",
            mip.flags
        );
        return Ok(None);
    }
    let path = match db.tfc_index.get(&tfc_stem.to_ascii_lowercase()) {
        Some(p) => p.clone(),
        None => {
            eprintln!("  \x1b[33mtfc\x1b[0m: '{tfc_stem}.tfc' not in --game-root index");
            return Ok(None);
        }
    };
    let mut f = File::open(&path)?;
    f.seek(SeekFrom::Start(mip.offset_in_file as u64))?;
    let mut buf = vec![0u8; mip.size_on_disk as usize];
    f.read_exact(&mut buf)?;
    Ok(Some(buf))
}

pub struct Texture2DSer;

impl NativeSerializer for Texture2DSer {
    fn class_name(&self) -> &'static str {
        "Texture2D"
    }

    fn read(&self, ctx: &NativeReadCtx) -> Result<NativeRead> {
        let mut payload = Texture2DPayload::parse_bytes(ctx.blob, ctx.ver)?;

        payload.format_label = prop_enum_label(ctx.props, "Format").map(str::to_string);
        payload.tfc_name = prop_string_or_name(ctx.props, "TextureFileCacheName", ctx.pak)
            .filter(|s| s != "None" && !s.is_empty());

        if let (Some(db), Some(tfc_stem)) = (ctx.db, payload.tfc_name.as_deref()) {
            for mip in payload.mips.iter_mut() {
                if matches!(mip.source, MipSource::Tfc { .. }) {
                    match resolve_tfc_payload(mip, tfc_stem, db)? {
                        Some(bytes) => {
                            mip.data = bytes;
                            mip.source = MipSource::Tfc {
                                stem_lc: tfc_stem.to_ascii_lowercase(),
                            };
                        }
                        None => {
                            mip.source = MipSource::Missing;
                        }
                    }
                }
            }
        }

        Ok(NativeRead::just(NativePayload::Texture2D(payload)))
    }

    fn emit_external(
        &self,
        payload: &NativePayload,
        dir: &Path,
        stem: &str,
    ) -> Result<Vec<PathBuf>> {
        let p = match payload {
            NativePayload::Texture2D(p) => p,
            _ => return Ok(Vec::new()),
        };

        let pf = match p
            .format_label
            .as_deref()
            .and_then(PixelFormat::from_pf_label)
        {
            Some(pf) => pf,
            None => {
                eprintln!(
                    "  \x1b[33mtex\x1b[0m: unmapped pixel format '{}' for {stem}; no .dds emitted",
                    p.format_label.as_deref().unwrap_or("?")
                );
                return Ok(Vec::new());
            }
        };

        let mut dds_mips: Vec<DdsMip> = p
            .mips
            .iter()
            .filter(|m| !m.data.is_empty() && m.size_x > 0 && m.size_y > 0)
            .map(|m| DdsMip {
                width: m.size_x as u32,
                height: m.size_y as u32,
                data: m.data.clone(),
            })
            .collect();

        if dds_mips.is_empty() {
            eprintln!(
                "  \x1b[33mtex\x1b[0m: no resolvable mips for {stem} (TFC '{}'); no .dds emitted",
                p.tfc_name.as_deref().unwrap_or("?")
            );
            return Ok(Vec::new());
        }

        dds_mips.sort_by_key(|m| std::cmp::Reverse(m.width as u64 * m.height as u64));

        let dds = Dds {
            format: pf,
            mips: dds_mips,
        };
        let bytes = dds.encode()?;
        let dds_path = dir.join(format!("{stem}.dds"));
        File::create(&dds_path)?.write_all(&bytes)?;

        println!(
            "  \x1b[36mtexture\x1b[0m → \x1b[32m{}\x1b[0m  ({} mips, {})",
            dds_path.display(),
            dds.mips.len(),
            pf.as_pf_label(),
        );
        Ok(vec![dds_path])
    }
}

#[allow(dead_code)]
fn write_indirect_mips<W: Write + Seek>(w: &mut W, mips: &[Mip]) -> Result<()> {
    w.write_i32::<LittleEndian>(mips.len() as i32)?;
    for m in mips {
        w.write_u32::<LittleEndian>(m.flags)?;
        w.write_i32::<LittleEndian>(m.element_count)?;
        w.write_i32::<LittleEndian>(m.size_on_disk)?;
        w.write_i32::<LittleEndian>(m.offset_in_file)?;
        if m.flags & BULKDATA_STORE_IN_SEPARATE_FILE == 0 && !m.data.is_empty() {
            w.write_all(&m.data)?;
        }
        w.write_i32::<LittleEndian>(m.size_x)?;
        w.write_i32::<LittleEndian>(m.size_y)?;
    }
    Ok(())
}
