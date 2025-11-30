use std::{env, fs::{self, File}, io::{BufReader, BufWriter, Cursor, Read, Result, Seek, SeekFrom, Write}, path::Path, process::exit};

use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};

use ron::{ser::{to_string_pretty, PrettyConfig}};
use upkreader::parse_upk;

use crate::{upkdecompress::{decompress_chunk, CompressedChunk, CompressionMethod}, upkreader::UpkHeader};

mod upkreader;
mod upkdecompress;
mod fontmod;

fn extract_from_ron(ron_path: &str, ron_class: &str) -> String {
    let ron_file = fs::read_to_string(ron_path).unwrap_or_else(|_| panic!("File `{}` not found", ron_path));
    
    let fmt = format!("{ron_class}(");
    let start = ron_file.find(&fmt).unwrap_or_else(|| panic!("`{}` not found in file", ron_class));
    
    let mut depth = 1;
    let mut end = start + fmt.len();

    for (i, c) in ron_file[end..].char_indices() {
        match c {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    end += i + 1;
                    break;
                }
            }
            _ => {}
        }
    }

    if depth != 0 {
        panic!("Something wrong, when parsing ron file for `{ron_class}`");
    }

    ron_file[start..end].to_string()
}

fn fontext(filepath: &str)
{
    let path = Path::new(filepath);
    let mut file = match File::open(path)
    {
        Ok(f) => f,
        Err(e) =>
        {
            eprintln!("Failed to open {}", e);
            return;
        }
    };

    fontmod::extract(&mut file);

}

fn upk_header_cursor(path: &str) -> Result<(Cursor<Vec<u8>>, upkreader::UpkHeader)>
{

    let path = Path::new(path);

    let file = File::open(path)?;

    let mut reader = BufReader::new(file);

    let header = upkreader::upk_read_header(&mut reader)?;
    println!("{}", header);

    if header.compression != CompressionMethod::None 
    {
        let mut chunks = Vec::new();
        if header.compressed_chunks != 0 {
            for _ in 0..header.compressed_chunks {
                chunks.push(CompressedChunk{
                    decompressed_offset: reader.read_u32::<LittleEndian>()?,
                    decompressed_size: reader.read_u32::<LittleEndian>()?,
                    compressed_offset: reader.read_u32::<LittleEndian>()?,
                    compressed_size: reader.read_u32::<LittleEndian>()?,
                });
            }
            println!("Compressed chunks: {:?}", chunks);
        }
        let mut dec_data: Vec<u8> = Vec::new(); 
        if header.compression == CompressionMethod::Zlo {
            for chunk in chunks {
                reader.seek(SeekFrom::Start(chunk.compressed_offset as u64))?;
                dec_data = decompress_chunk(
                    &mut reader,
                    chunk.compressed_size as usize,
                    header.compression,
                    chunk.decompressed_size as usize
                )?;
            }
        }

        reader.seek(SeekFrom::Start(0))?;
        let mut header_raw_bytes = vec![0u8; 0x79];
        reader.read_exact(&mut header_raw_bytes)?;

        let file = File::create("../test.upk")?;
        let mut writer = BufWriter::new(file);

        writer.write_all(&header_raw_bytes)?;
        writer.write_u32::<LittleEndian>(0)?;
        writer.write_u32::<LittleEndian>(0)?;
        writer.write_all(&dec_data)?;

        exit(-1);
    }

    reader.seek(SeekFrom::Start(0))?;
    let mut buf = Vec::new();
    reader.read_to_end(&mut buf)?;
    Ok((Cursor::new(buf), header))
}

fn getlist(path: &str) -> Result<()>
{
    let (cursor, header): (Cursor<Vec<u8>>, upkreader::UpkHeader) = upk_header_cursor(path)?;
    let mut cur: Cursor<&Vec<u8>> = Cursor::new(cursor.get_ref());

    let pak = parse_upk(&mut cur, &header)?;
    let list = upkreader::list_full_obj_paths(&pak);
    for (i, path) in list.iter().enumerate()
    {
        println!("#{} {}", i, path);
    }

    Ok(())
}

fn el(path: &str, ron_path: &str) -> Result<()>
{
    if path.is_empty()
    {
        println!("No object file provided");
        exit(-1);
    }

    if ron_path.is_empty()
    {
        println!("No `.ron` file provided");
        exit(-1);
    }

    let upk: upkreader::UPKPak = ron::from_str(&extract_from_ron(ron_path, "UPKPak")).expect("RON Error");


    let el_data = fs::read(path)?;
    let mut cursor = Cursor::new(&el_data);

    loop
    {
        let _tag = upkreader::read_proptag(&mut cursor, &upk.name_table)?;

        match _tag
        {
            None => break,
            Some(tag) =>
            {
                let v = upkreader::parse_prop_val(&mut cursor, &tag, &upk.name_table)?;
                let pn = &upk.name_table[tag.name_idx as usize];

                println!("{} = {}", pn, v);
            }  
        }
    }
    Ok(())
}

fn dump_names(upk_path: &str, mut output_path: &str) -> Result<()>
{

    if output_path.is_empty()
    {
        output_path = "names_table.txt";
    }

    let (cursor, header): (Cursor<Vec<u8>>, upkreader::UpkHeader) = upk_header_cursor(upk_path)?;
    let mut cur: Cursor<&Vec<u8>> = Cursor::new(cursor.get_ref());
    cur.seek(SeekFrom::Start(header.name_offset as u64))?;

    println!("Names: (count = {})", header.name_count);

    let nt_file = File::create(Path::new(output_path))?;
    let mut writer = BufWriter::new(nt_file);

    for i in 0..header.name_count
    {
        // if i == 0
        // {
        //     println!("Name[{}]: NULL", i);
        //     writeln!(writer, "NULL")?;
        //     continue;
        // }
        let s = upkreader::read_name(&mut cur)?;
        println!("Name[{}]: {}", i, s.name);
        writeln!(writer, "{}", s.name)?;
    }

    Ok(())
}

fn extract_file(upk_path: &str, path: &str, mut output_dir: &str, all: bool) -> Result<()> {
    
    if output_dir.is_empty()
    {
        output_dir = "output";
    }

    let output_dir_path = Path::new(output_dir);
    
    let filename = Path::new(upk_path).file_stem().unwrap();

    
    let pbuf = output_dir_path.join(filename);
    let dir_path: &Path = pbuf.as_path();

    let (mut cursor, header): (Cursor<Vec<u8>>, upkreader::UpkHeader) = upk_header_cursor(upk_path)?;
    let mut cur: Cursor<&Vec<u8>> = Cursor::new(cursor.get_ref());
    let up = upkreader::parse_upk(&mut cur, &header)?;

    if !dir_path.exists() {
        std::fs::create_dir_all(dir_path)?;
    }
    
    let mut data_file = File::create(pbuf.with_extension("ron"))?;

    let pretty = PrettyConfig::new().struct_names(true);

    let s = to_string_pretty(&header, pretty.clone()).expect("Fail");
    writeln!(data_file, "{s}")?;

    let s = to_string_pretty(&up, pretty).expect("Fail");
    writeln!(data_file, "{s}")?;

    upkreader::extract_by_name(&mut cursor, &up, path, dir_path, all)?;

    Ok(())
}

fn pack_upk(_ron_path: &str) -> Result<()> {
    unimplemented!("For now");
}

fn main() -> Result<()> 
{

    let args: Vec<String> = env::args().collect();

    if args.len() <= 1
    {
        println!("No args!");
        exit(0);
    }

    let key = &args[1];
    let ac: Vec<&str> = args.iter().skip(2).map(|s| s.as_str()).collect();
    let arg = |i: usize| ac.get(i).copied().unwrap_or("");

    match key.as_str()
    {
        "fontext"       => fontext(arg(0)),
        "upkHeader"     => { upk_header_cursor(arg(0))?; }
        "element"       => el(arg(0), arg(1))?,
        "list"          => getlist(arg(0))?,
        "names"         => dump_names(arg(0), arg(1))?,
        "extract"       => extract_file(arg(0), arg(1), arg(2), false)?,
        "extractall"    => extract_file(arg(0), "", arg(1), true)?,
        "pack"          => pack_upk(arg(0))?,
        _               => println!("unknown")
    }
    Ok(())
}
