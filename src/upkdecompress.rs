use std::{io::{self, Error, ErrorKind, Read, Result, Seek, SeekFrom}};

use byteorder::{LittleEndian, ReadBytesExt};

use crate::upkreader::PACKAGE_TAG;

pub const CHUNK_SIZE: u32 = 131072;

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

        let tag = reader.read_u32::<LittleEndian>()?;
        let mut chunk_size = reader.read_u32::<LittleEndian>()?;
        let mut _summary = reader.read_u32::<LittleEndian>()?;
        let mut summary_2 = reader.read_u32::<LittleEndian>()?;

        let bswap: bool = tag != PACKAGE_TAG;

        if bswap {
            if tag.swap_bytes() != PACKAGE_TAG {
                return Err(Error::new(ErrorKind::InvalidData, "Invalid tag."));
            } else {
                _summary = _summary.swap_bytes();
                summary_2 = summary_2.swap_bytes();
                chunk_size = chunk_size.swap_bytes();
            }
        }

        if chunk_size == PACKAGE_TAG {
            chunk_size = CHUNK_SIZE;
        }

        let total_count = summary_2.div_ceil(chunk_size);

        let mut raw_chunks = Vec::new();

        for _ in 0..total_count {
            let mut compressed_size = reader.read_u32::<LittleEndian>()?;
            let mut decompressed_size = reader.read_u32::<LittleEndian>()?;
            if bswap {
                compressed_size = compressed_size.swap_bytes();
                decompressed_size = decompressed_size.swap_bytes();
            }
            raw_chunks.push((compressed_size, decompressed_size));
        }
    
        let mut rchunk_data: Vec<u8> = Vec::new();

        for rchunk in raw_chunks {
            let mut compressed_data = vec![0u8; rchunk.0 as usize];
            reader.read_exact(&mut compressed_data)?;

            let chunk_data = decompress_chunk(
                compressed_data,
                mode,
                rchunk.1 as usize
            )?;

            rchunk_data.extend_from_slice(&chunk_data);
        }
        
        if chunk.decompressed_size as usize > rchunk_data.len() {
            rchunk_data.resize(chunk.decompressed_size as usize, 0);
        }

        dec_data.push(rchunk_data);
    }

    Ok(dec_data)
}

pub fn decompress_chunk(
    compressed: Vec<u8>,
    mode: CompressionMethod,
    expected_decompress_size: usize
) -> Result<Vec<u8>> {
    let mut out = vec![0u8; expected_decompress_size];
    let out_len = expected_decompress_size;

    match mode {
        CompressionMethod::Lzo => {
            lzo1x::decompress(&compressed, &mut out).unwrap();
             
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

