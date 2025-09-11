use std::io::{Cursor, Read, Result};

use byteorder::{LittleEndian, ReadBytesExt};
use flate2::read::ZlibDecoder;

#[derive(Debug)]
pub struct CompressedChunkBlock
{
    pub compressed_size: u32,
    decompressed_size: u32
}

#[derive(Debug)]
pub struct CompressedChunkHeader
{
    sig: u32,
    block_size: u32,
    pub compressed_size: u32,
    decompressed_size: u32,
    num_blocks: u32,
    blocks: Vec<CompressedChunkBlock>
}

#[derive(Debug)]
pub struct CompressedChunk
{
    decompressed_offset: u32,
    decompressed_size: u32,
    compressed_offset: u32,
    pub compressed_size: u32
}

pub fn parse_chunk_header(data: &[u8]) -> Result<CompressedChunkHeader>
{
    let mut cursor = Cursor::new(data);

    let sig = cursor.read_u32::<LittleEndian>()?;
    let bs  = cursor.read_u32::<LittleEndian>()?;
    let cs  = cursor.read_u32::<LittleEndian>()?;
    let dcs = cursor.read_u32::<LittleEndian>()?;
    let nb  = cursor.read_u32::<LittleEndian>()?;

    let mut blocks = Vec::new();
    for _ in 0..nb
    {    
        let cs  = cursor.read_u32::<LittleEndian>()?;
        let dcs = cursor.read_u32::<LittleEndian>()?;
        blocks.push(
            CompressedChunkBlock
            {
                compressed_size: cs,
                decompressed_size: dcs
            }
        );
    }

    Ok(
        CompressedChunkHeader
        {
            sig,
            block_size: bs,
            compressed_size: cs,
            decompressed_size: dcs,
            num_blocks: nb,
            blocks
        }
    )
}

pub fn decompress_chunk(header: &CompressedChunkHeader, compressed_data: &[u8]) -> Result<Vec<u8>>
{
    let mut res = Vec::with_capacity(header.compressed_size as usize);
    let mut offset = 0;

    for block in &header.blocks
    {
        let end = offset + block.compressed_size as usize;
        let slice = &compressed_data[offset..end];
        let mut decoder = ZlibDecoder::new(slice);
        let mut buf = vec![0u8; block.decompressed_size as usize];
        decoder.read_exact(&mut buf)?;
        res.extend_from_slice(&buf);
        offset = end;
    }

    Ok(res)
}

pub fn read_compressed_chunks<R: Read>(reader: &mut R, count: usize) -> Result<Vec<CompressedChunk>>
{
    let mut chunks = Vec::with_capacity(count);

    for _ in 0..count
    {
        let decompressed_offset = reader.read_u32::<LittleEndian>()?;
        let decompressed_size = reader.read_u32::<LittleEndian>()?;
        let compressed_offset = reader.read_u32::<LittleEndian>()?;
        let compressed_size = reader.read_u32::<LittleEndian>()?;

        chunks.push(
            CompressedChunk
            {
                decompressed_offset,
                decompressed_size,
                compressed_offset,
                compressed_size
            }
        );
    }

    Ok(chunks)
}

