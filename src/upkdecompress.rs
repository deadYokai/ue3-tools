use std::io::{Error, ErrorKind, Read, Result, Seek, SeekFrom};

use byteorder::{LittleEndian, ReadBytesExt};

use crate::upkreader;

#[derive(Debug)]
pub struct CompressedChunkBlock
{
    pub compressed_size: u32,
    pub decompressed_size: u32
}

#[derive(Debug)]
pub struct CompressedChunkHeader
{
    pub sig: u32,
    pub block_size: u32,
    pub compressed_size: u32,
    pub decompressed_size: u32,
    pub num_blocks: u32,
    pub blocks: Vec<CompressedChunkBlock>
}

#[derive(Debug)]
pub struct CompressedChunk
{
    pub decompressed_offset: u32,
    pub decompressed_size: u32,
    pub compressed_offset: u32,
    pub compressed_size: u32
}

pub fn parse_chunk_header<R: Read + Seek>(reader: &mut R, header: &upkreader::UpkHeader) -> Result<Option<(CompressedChunkHeader, u64)>> { 
    let sig = reader.read_u32::<LittleEndian>()?;
    let block_size = reader.read_u32::<LittleEndian>()?;
    let compressed_size = reader.read_u32::<LittleEndian>()?;
    let decompressed_size = reader.read_u32::<LittleEndian>()?;
    let num_blocks = reader.read_u32::<LittleEndian>()?;

    if header.sign != sig {
        return Err(Error::new(ErrorKind::InvalidData, format!("Wrong signature: {:x?} != {:x?}", header.sign, sig)));
    }

    if num_blocks > 10000 {
        return Err(Error::new(ErrorKind::InvalidData, format!("Strange num_blocks value: {}", num_blocks)));
    }

    let mut blocks = Vec::with_capacity(num_blocks as usize);
    for _ in 0..num_blocks {
        let compressed_size = reader.read_u32::<LittleEndian>()?;
        let decompressed_size = reader.read_u32::<LittleEndian>()?;
        blocks.push(CompressedChunkBlock{
            compressed_size,
            decompressed_size
        });
    }

    let header = CompressedChunkHeader{
        sig,
        block_size,
        compressed_size,
        decompressed_size,
        num_blocks,
        blocks
    };

    let data_offset = reader.stream_position()?;
    Ok(Some((header, data_offset)))
}

