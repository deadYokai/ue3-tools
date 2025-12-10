use std::{fs::{self, File}, io::{BufReader, BufWriter, Cursor, Read, Result, Seek, SeekFrom, Write}, path::Path, process::exit};
use byteorder::{LittleEndian, ReadBytesExt};
use ron::{ser::{to_string_pretty, PrettyConfig}};
use upkreader::parse_upk;
use crate::{upkdecompress::{decompress_chunk, CompressedChunk, CompressionMethod}, upkprops::parse_property};
use clap::{Parser, Subcommand};

mod upkreader;
mod upkdecompress;
mod upkprops;
mod fontmod;

// stupid ron parser
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

fn upk_header_cursor(path: &str) -> Result<(Cursor<Vec<u8>>, upkreader::UpkHeader)>
{

    let path = Path::new(path);

    let file = File::open(path)?;

    let mut reader = BufReader::new(file);

    let header = upkreader::upk_read_header(&mut reader)?;
    println!("{}", header);
     
    if header.compression != CompressionMethod::None 
    {

        if header.compressed_chunks != 0 {
            let mut chunks = Vec::new();
            for _ in 0..header.compressed_chunks {
                chunks.push(CompressedChunk{
                    decompressed_offset: reader.read_u32::<LittleEndian>()?,
                    decompressed_size: reader.read_u32::<LittleEndian>()?,
                    compressed_offset: reader.read_u32::<LittleEndian>()?,
                    compressed_size: reader.read_u32::<LittleEndian>()?,
                });
            }
            println!("Compressed chunks: {:?}", chunks);

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

            let file = File::create("../test.upk")?;
            let mut writer = BufWriter::new(file);
            writer.write_all(&dec_data)?;
        }

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

    while let Some(prop) = parse_property(&mut cursor, &upk)? {
            println!("{:?}", prop);
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

fn swffont(path: &str, ron_path: &str) -> Result<()> {
    unimplemented!("WIP")
}

#[derive(Parser)]
#[command(name = "ue3-tools")]
#[command(about = "Unreal3 upk stuff")]
struct Cli {
    #[command(subcommand)]
    command: Commands
}

#[derive(Subcommand)]
enum Commands {
    Swffont {
        path: String,
        ron_path: String
    },

    #[command(about = "Print header info of upk file")]
    UpkHeader {
        path: String
    },

    #[command(about = "Print elements in object")]
    Elements {
        path: String,
        ron_path: String
    },

    #[command(about = "Print list of objects in upk file")]
    List {
        path: String
    },

    #[command(about = "Print or extract names in upk file")]
    Names {
        path: String,
        output_path: Option<String>
    },

    #[command(about = "Extract specific object from upk")]
    Extract {
        upk_path: String,
        path: String,
        output_dir: Option<String>
    },

    #[command(about = "Extract all objects from upk")]
    Extractall {
        upk_path: String,
        output_dir: Option<String>
    },

    Pack {
        ron_path: String
    }
}

fn main() -> Result<()> 
{
    let cli = Cli::parse();

    match cli.command {
        Commands::Swffont { path, ron_path } => swffont(&path, &ron_path)?,
        Commands::UpkHeader { path } => { upk_header_cursor(&path)?; },
        Commands::Elements { path, ron_path } => el(&path, &ron_path)?,
        Commands::List { path } => getlist(&path)?,
        Commands::Names { path, output_path } => { 
            let out = output_path.as_deref().unwrap_or("");
            dump_names(&path, out)?
        },
        Commands::Extract { upk_path, path, output_dir } => {
            let out = output_dir.as_deref().unwrap_or("");
            extract_file(&upk_path, &path, out, false)?
        },
        Commands::Extractall { upk_path, output_dir } => {
            let out = output_dir.as_deref().unwrap_or("");
            extract_file(&upk_path, "", out, true)?
        },
        Commands::Pack { .. } => unimplemented!()
    }

    Ok(())
}
