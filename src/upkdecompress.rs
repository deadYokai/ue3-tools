use std::{io::{self, Error, Read, Result, Seek, SeekFrom}, ptr};
use lzo_sys::{lzo1x::lzo1x_decompress, lzoconf::LZO_E_OK};

#[derive(Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize, Copy, Clone)]
#[repr(u32)]
pub enum CompressionMethod {
    None,
    Zlib,
    Lzo,
    Lzx = 4
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
            2 => Ok(CompressionMethod::Lzo),
            4 => Ok(CompressionMethod::Lzx),
            _ => Err(())
        }
    }
}

pub fn upk_decompress<R: Read + Seek>(
    mut reader: R,
    mode: CompressionMethod,
    chunks: &Vec<CompressedChunk>
) -> Result<Vec<Vec<u8>>> {

    let mut dec_data = Vec::new();

    for chunk in chunks {
        reader.seek(SeekFrom::Start(chunk.compressed_offset as u64))?;

        let mut compressed_data = vec![0u8; chunk.compressed_size as usize];
        reader.read_exact(&mut compressed_data)?;

        let chunk_data = decompress_chunk(
            compressed_data,
            chunk.compressed_size as usize,
            mode,
            chunk.decompressed_size as usize
        )?;

        dec_data.push(chunk_data);
    }

    Ok(dec_data)
}

pub fn decompress_chunk(
    compressed: Vec<u8>,
    compressed_size: usize,
    mode: CompressionMethod,
    expected_decompress_size: usize
) -> Result<Vec<u8>> {
    let mut out = vec![0u8; expected_decompress_size];
    let mut out_len = expected_decompress_size;

    match mode {
        CompressionMethod::Lzo => {
            let result = unsafe {
                lzo1x_decompress(
                    compressed.as_ptr(),
                    compressed_size,
                    out.as_mut_ptr(),
                    &mut out_len,
                    ptr::null_mut()
                )
            };

            if result != LZO_E_OK {
                return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!("LZO decompression failed (code {})", result)
                ));
            }

            if out_len > expected_decompress_size {
                return Err(Error::new(
                        io::ErrorKind::InvalidData,
                        format!(
                            "LZO decompression failed. Size = {}, expected = {}", 
                            out_len, expected_decompress_size
                        )
                ));
            }

            if out_len < expected_decompress_size {
                out[out_len..expected_decompress_size].fill(0);
            }
        },
        _ => unimplemented!()
    }

    Ok(out)
}

