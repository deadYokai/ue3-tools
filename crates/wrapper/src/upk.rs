use super::patch_fmt::CdoPatch;
use std::io::{self, Cursor, Read, Seek, SeekFrom};

const PACKAGE_TAG: u32 = 0x9E2A83C1;

struct Header {
    p_ver: i16,
    name_count: i32,
    name_offset: i32,
    export_count: i32,
    export_offset: i32,
}

impl Header {
    fn read(data: &[u8]) -> io::Result<Self> {
        let mut c = Cursor::new(data);
        let tag = read_u32(&mut c)?;
        if tag != PACKAGE_TAG {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "bad UPK magic"));
        }
        let p_ver = read_i16(&mut c)?;
        let _l_ver = read_i16(&mut c)?;
        let _header_size = read_i32(&mut c)?;

        let path_len = read_i32(&mut c)?;
        let skip = if path_len < 0 {
            (-path_len * 2) as u64
        } else {
            path_len as u64
        };
        c.seek(SeekFrom::Current(skip as i64))?;

        let _pak_flags = read_u32(&mut c)?;
        let name_count = read_i32(&mut c)?;
        let name_offset = read_i32(&mut c)?;
        let export_count = read_i32(&mut c)?;
        let export_offset = read_i32(&mut c)?;

        Ok(Self {
            p_ver,
            name_count,
            name_offset,
            export_count,
            export_offset,
        })
    }
}

#[derive(Clone)]
struct ExportEntry {
    name_idx: usize,
    outer_index: i32,
    serial_size: i32,
    serial_offset: i32,
    serial_size_fpos: usize,
    serial_offset_fpos: usize,
}

pub(super) fn apply_cdo_patches(raw: &[u8], patches: &[CdoPatch]) -> io::Result<Vec<u8>> {
    let hdr = Header::read(raw)?;
    let names = read_name_table(raw, &hdr)?;
    let exports = read_export_table(raw, &hdr)?;

    let path_map: std::collections::HashMap<String, usize> = exports
        .iter()
        .enumerate()
        .map(|(i, e)| {
            let path = build_export_path(&names, &exports, i);
            (path.to_ascii_lowercase(), i)
        })
        .collect();

    let mut replacements: std::collections::HashMap<usize, Vec<u8>> =
        std::collections::HashMap::new();

    for patch in patches {
        let key = patch.object_path.to_ascii_lowercase();
        let idx = path_map.get(&key).or_else(|| {
            path_map
                .iter()
                .find(|(k, _)| k.ends_with(&*key) || key.ends_with(&**k))
                .map(|(_, v)| v)
        });
        if let Some(&idx) = idx {
            replacements.insert(idx, patch.data.clone());
        }
    }

    if replacements.is_empty() {
        return Ok(raw.to_vec());
    }

    rebuild(raw, &exports, &replacements)
}

fn read_name_table(raw: &[u8], hdr: &Header) -> io::Result<Vec<String>> {
    let mut c = Cursor::new(raw);
    c.seek(SeekFrom::Start(hdr.name_offset as u64))?;
    let mut names = Vec::with_capacity(hdr.name_count as usize);
    for _ in 0..hdr.name_count {
        let len = read_i32(&mut c)?;
        let s = if len < 0 {
            let n = (-len) as usize;
            let mut chars = Vec::with_capacity(n);
            for _ in 0..n {
                chars.push(read_u16(&mut c)?);
            }
            if chars.last() == Some(&0) {
                chars.pop();
            }
            String::from_utf16_lossy(&chars)
        } else {
            let mut b = vec![0u8; len as usize];
            c.read_exact(&mut b)?;
            if b.last() == Some(&0) {
                b.pop();
            }
            String::from_utf8_lossy(&b).into_owned()
        };
        let _flags = read_u64(&mut c)?;
        names.push(s);
    }
    Ok(names)
}

fn read_export_table(raw: &[u8], hdr: &Header) -> io::Result<Vec<ExportEntry>> {
    let mut c = Cursor::new(raw);
    c.seek(SeekFrom::Start(hdr.export_offset as u64))?;
    let mut out = Vec::with_capacity(hdr.export_count as usize);

    for _ in 0..hdr.export_count {
        let _class = read_i32(&mut c)?;
        let _super = read_i32(&mut c)?;
        let outer = read_i32(&mut c)?;

        let name_idx = read_i32(&mut c)? as usize;
        let _name_inst = read_i32(&mut c)?;

        let _archetype = read_i32(&mut c)?;
        let _obj_flags = read_u64(&mut c)?;

        let sz_fpos = c.position() as usize;
        let serial_size = read_i32(&mut c)?;
        let off_fpos = c.position() as usize;
        let serial_offset = read_i32(&mut c)?;

        if hdr.p_ver < 543 {
            let n = read_i32(&mut c)? as usize;
            for _ in 0..n {
                c.seek(SeekFrom::Current(12))?;
            }
        }

        let _export_flags = read_u32(&mut c)?;
        let gen_cnt = read_i32(&mut c)? as i64;
        c.seek(SeekFrom::Current(gen_cnt * 4))?;
        c.seek(SeekFrom::Current(20))?;
        out.push(ExportEntry {
            name_idx,
            outer_index: outer,
            serial_size,
            serial_offset,
            serial_size_fpos: sz_fpos,
            serial_offset_fpos: off_fpos,
        });
    }
    Ok(out)
}

fn build_export_path(names: &[String], exports: &[ExportEntry], idx: usize) -> String {
    let mut parts = Vec::new();
    let mut cur = idx;
    loop {
        let name = names
            .get(exports[cur].name_idx)
            .map(|s| s.as_str())
            .unwrap_or("<bad>");
        parts.push(name.to_owned());
        let outer = exports[cur].outer_index;
        if outer <= 0 {
            break;
        }
        cur = (outer - 1) as usize;
        if cur >= exports.len() {
            break;
        }
    }
    parts.reverse();
    parts.join(".")
}

fn rebuild(
    raw: &[u8],
    exports: &[ExportEntry],
    replacements: &std::collections::HashMap<usize, Vec<u8>>,
) -> io::Result<Vec<u8>> {
    let mut order: Vec<usize> = exports
        .iter()
        .enumerate()
        .filter(|(_, e)| e.serial_size > 0)
        .map(|(i, _)| i)
        .collect();
    order.sort_by_key(|&i| exports[i].serial_offset);

    let data_start = order
        .first()
        .map(|&i| exports[i].serial_offset as usize)
        .unwrap_or(raw.len());
    let data_end = order
        .last()
        .map(|&i| (exports[i].serial_offset + exports[i].serial_size) as usize)
        .unwrap_or(raw.len());

    let mut out = raw[..data_start].to_vec();
    let mut new_offsets: Vec<(i32, i32)> = exports
        .iter()
        .map(|e| (e.serial_offset, e.serial_size))
        .collect();

    let mut cur_off = data_start;
    for &ei in &order {
        let (blob, sz): (&[u8], usize) = if let Some(nb) = replacements.get(&ei) {
            (nb.as_slice(), nb.len())
        } else {
            let s = exports[ei].serial_offset as usize;
            let n = exports[ei].serial_size as usize;
            (&raw[s..s + n], n)
        };
        new_offsets[ei] = (cur_off as i32, sz as i32);
        out.extend_from_slice(blob);
        cur_off += sz;
    }

    if data_end < raw.len() {
        out.extend_from_slice(&raw[data_end..]);
    }

    for (ei, exp) in exports.iter().enumerate() {
        let (new_off, new_sz) = new_offsets[ei];
        let sz_p = exp.serial_size_fpos;
        let off_p = exp.serial_offset_fpos;
        if sz_p + 4 <= out.len() {
            out[sz_p..sz_p + 4].copy_from_slice(&new_sz.to_le_bytes());
        }
        if off_p + 4 <= out.len() {
            out[off_p..off_p + 4].copy_from_slice(&new_off.to_le_bytes());
        }
    }

    Ok(out)
}

fn read_u32(c: &mut Cursor<&[u8]>) -> io::Result<u32> {
    let mut b = [0u8; 4];
    c.read_exact(&mut b)?;
    Ok(u32::from_le_bytes(b))
}
fn read_u64(c: &mut Cursor<&[u8]>) -> io::Result<u64> {
    let mut b = [0u8; 8];
    c.read_exact(&mut b)?;
    Ok(u64::from_le_bytes(b))
}
fn read_i32(c: &mut Cursor<&[u8]>) -> io::Result<i32> {
    let mut b = [0u8; 4];
    c.read_exact(&mut b)?;
    Ok(i32::from_le_bytes(b))
}
fn read_i16(c: &mut Cursor<&[u8]>) -> io::Result<i16> {
    let mut b = [0u8; 2];
    c.read_exact(&mut b)?;
    Ok(i16::from_le_bytes(b))
}
fn read_u16(c: &mut Cursor<&[u8]>) -> io::Result<u16> {
    let mut b = [0u8; 2];
    c.read_exact(&mut b)?;
    Ok(u16::from_le_bytes(b))
}
