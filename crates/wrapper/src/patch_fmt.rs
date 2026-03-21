use flate2::read::ZlibDecoder;
use std::io::{self, Cursor, Read};

const BLOCK_SIZE: usize = 0x20000;

#[derive(Clone)]
pub(super) struct CdoPatch {
    pub(super) object_path: String,
    pub(super) data: Vec<u8>,
}

pub(super) fn load_patch_bin(bin: &[u8]) -> io::Result<Vec<CdoPatch>> {
    if bin.len() < 8 {
        return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "too short"));
    }
    let unc_total = u32::from_le_bytes(bin[0..4].try_into().unwrap()) as usize;
    let n_blocks = (unc_total + BLOCK_SIZE - 1) / BLOCK_SIZE;
    let hdr_end = 8 + n_blocks * 8;

    let mut unc = Vec::with_capacity(unc_total);
    let mut pos = hdr_end;
    for i in 0..n_blocks {
        let h = 8 + i * 8;
        let cs = u32::from_le_bytes(bin[h..h + 4].try_into().unwrap()) as usize;
        if pos + cs > bin.len() {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "block out of range",
            ));
        }
        let mut dec = ZlibDecoder::new(&bin[pos..pos + cs]);
        let mut blk = Vec::new();
        dec.read_to_end(&mut blk)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        unc.extend_from_slice(&blk);
        pos += cs;
    }

    extract_cdo_patches(&mut Cursor::new(unc))
}

fn extract_cdo_patches(r: &mut Cursor<Vec<u8>>) -> io::Result<Vec<CdoPatch>> {
    read_ue3_string(r)?;

    skip_string_array(r)?;
    skip_empty_array(r, "Exports")?;
    skip_empty_array(r, "Imports")?;
    skip_patch_array(r)?;

    let n = read_i32(r)? as usize;
    let mut out = Vec::with_capacity(n);
    for _ in 0..n {
        let name = read_ue3_string(r)?;
        let data = read_byte_array(r)?;
        out.push(CdoPatch {
            object_path: name,
            data,
        });
    }
    Ok(out)
}

fn read_i32(r: &mut Cursor<Vec<u8>>) -> io::Result<i32> {
    let mut b = [0u8; 4];
    r.read_exact(&mut b)?;
    Ok(i32::from_le_bytes(b))
}

fn read_ue3_string(r: &mut Cursor<Vec<u8>>) -> io::Result<String> {
    let len = read_i32(r)?;
    if len == 0 {
        return Ok(String::new());
    }
    if len > 0 {
        let mut b = vec![0u8; len as usize];
        r.read_exact(&mut b)?;
        if b.last() == Some(&0) {
            b.pop();
        }
        Ok(String::from_utf8_lossy(&b).into_owned())
    } else {
        let count = (-len) as usize;
        let mut chars = Vec::with_capacity(count);
        for _ in 0..count {
            let mut b = [0u8; 2];
            r.read_exact(&mut b)?;
            chars.push(u16::from_le_bytes(b));
        }
        if chars.last() == Some(&0) {
            chars.pop();
        }
        Ok(String::from_utf16_lossy(&chars))
    }
}

fn read_byte_array(r: &mut Cursor<Vec<u8>>) -> io::Result<Vec<u8>> {
    let n = read_i32(r)?;
    if n <= 0 {
        return Ok(Vec::new());
    }
    let mut b = vec![0u8; n as usize];
    r.read_exact(&mut b)?;
    Ok(b)
}

fn skip_string_array(r: &mut Cursor<Vec<u8>>) -> io::Result<()> {
    let n = read_i32(r)? as usize;
    for _ in 0..n {
        read_ue3_string(r)?;
    }
    Ok(())
}

fn skip_empty_array(r: &mut Cursor<Vec<u8>>, name: &str) -> io::Result<()> {
    let n = read_i32(r)?;
    if n != 0 {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            format!("{name} array must be empty in this patch format"),
        ));
    }
    Ok(())
}

fn skip_patch_array(r: &mut Cursor<Vec<u8>>) -> io::Result<()> {
    let n = read_i32(r)? as usize;
    for _ in 0..n {
        read_ue3_string(r)?;
        read_byte_array(r)?;
    }
    Ok(())
}
