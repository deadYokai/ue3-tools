use std::{env, fs::File, io::{BufReader, Read, Seek, SeekFrom}, path::Path};

use upkdecompress::decompress_chunk;

mod upkreader;
mod upkdecompress;
mod fontmod;

fn main() 
{

    let args: Vec<String> = env::args().collect();

    let path = Path::new(&args[1]);

    let file = match File::open(path)
    {
        Ok(f) => f,
        Err(e) =>
        {
            eprintln!("Failed to open {}", e);
            return;
        }
    };

    let mut reader = BufReader::new(file);

    let header = match upkreader::upk_read_header(&mut reader)
    {
        Ok(h) =>
        {
            println!("{}", h);
            h
        }
        Err(e) =>
        {
            eprintln!("Err: {}", e);
            return;
        }
    };

    if let Err(e) = reader.seek(SeekFrom::Start(header.header_size as u64))
    {
        eprintln!("Failed to seek chunks: {}", e);
        return;
    }

    let mut chunk_header_buf = vec![0u8; 4096];

    if let Err(e) = reader.read_exact(&mut chunk_header_buf[..20])
    {
        return;
    }

    let num_blocks = u32::from_le_bytes(chunk_header_buf[16..20].try_into().unwrap());
    let total_header_size = 20 + (num_blocks as usize * 8);
    if let Err(e) = reader.read_exact(&mut chunk_header_buf[20..total_header_size])
    {
        return;
    }

    let chunk_header = match upkdecompress::parse_chunk_header(&chunk_header_buf[..total_header_size])
    {
        Ok(h) => h,
        Err(e) => return
    };

    let mut compressed_data = vec![0u8; chunk_header.compressed_size as usize];
    if let Err(e) = reader.read_exact(&mut compressed_data)
    {
        return;
    }

    let decompressed = match upkdecompress::decompress_chunk(&chunk_header, &compressed_data)
    {
        Ok(data) => data,
        Err(e) => return
    };

    println!("Decompressed size: {}", decompressed.len());
}
