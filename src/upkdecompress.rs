use std::{io::{self, BufReader, Read}, ptr};
use lzo_sys::{lzo1x::lzo1x_decompress_safe, lzoconf::LZO_E_OK};

#[derive(Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize, Copy, Clone)]
#[repr(u32)]
pub enum CompressionMethod {
    None,
    Zlib,
    Zlo,
    Zlx = 4
}

#[derive(Debug)]
pub struct CompressedChunk
{
    pub decompressed_offset: u32,
    pub decompressed_size: u32,
    pub compressed_offset: u32,
    pub compressed_size: u32
}

impl TryFrom<u32> for CompressionMethod {
    type Error = ();

    fn try_from(value: u32) -> std::result::Result<Self, Self::Error> {
        match value {
            0 => Ok(CompressionMethod::None),
            1 => Ok(CompressionMethod::Zlib),
            2 => Ok(CompressionMethod::Zlo),
            4 => Ok(CompressionMethod::Zlx),
            _ => Err(())
        }
    }
}

pub fn decompress_chunk<R: Read>(
    reader: &mut BufReader<R>,
    compressed_size: usize,
    mode: CompressionMethod,
    expected_decompress_size: usize
) -> io::Result<Vec<u8>> {
    let mut compressed = vec![0u8; compressed_size];
    reader.read_exact(&mut compressed)?;

    let mut out = vec![0u8; expected_decompress_size];
    let mut out_len = expected_decompress_size;

    println!("Compressed data (first 32 bytes): {:02x?}", &compressed[..32.min(compressed.len())]);
    println!("Sizes - compressed: {}, expected decompressed: {}", compressed_size, expected_decompress_size);

    if mode == CompressionMethod::Zlo {
        let result = unsafe {
            lzo1x_decompress_safe(
                compressed.as_ptr(),
                compressed.len(),
                out.as_mut_ptr(),
                &mut out_len,
                ptr::null_mut()
            )
        };

        if result != LZO_E_OK{
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("LZO decompression failed (code {})", result)
            ));
        }
    }

    Ok(out)
}

