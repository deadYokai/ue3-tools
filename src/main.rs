use std::{fs::{self, File}, io::{BufReader, BufWriter, Cursor, Read, Result, Seek, SeekFrom, Write}, path::Path, process::exit};
use byteorder::{LittleEndian, ReadBytesExt};
use ron::{ser::{to_string_pretty, PrettyConfig}};
use upkreader::parse_upk;
use crate::{upkdecompress::{upk_decompress, CompressedChunk, CompressionMethod}, upkreader::{get_obj_props, UPKPak, UpkHeader}};
use clap::{Parser, Subcommand};

mod upkreader;
mod upkpacker;
mod upkdecompress;
mod upkprops;
mod upkfont;

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
            let mut chunks = Vec::with_capacity(header.compressed_chunks as usize);

            for _ in 0..header.compressed_chunks {
                chunks.push(CompressedChunk{
                    decompressed_offset: reader.read_u32::<LittleEndian>()?,
                    decompressed_size: reader.read_u32::<LittleEndian>()?,
                    compressed_offset: reader.read_u32::<LittleEndian>()?,
                    compressed_size: reader.read_u32::<LittleEndian>()?,
                });
            }
            
            chunks.sort_by_key(|c| c.decompressed_offset);

            println!("Compressed chunks: {:?}", chunks);

            let dec_data = upk_decompress(&mut reader, header.compression, &chunks)
                .expect("Decompression error"); 
 
            let file = File::create("test.upk")?;
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

    let config = PrettyConfig::new().struct_names(true);

    let tup = (&filename.to_str().unwrap(), &upk_path, &header, &up);
    let s = to_string_pretty(&tup, config).expect("Fail");
    writeln!(data_file, "{s}")?;

    // let s = to_string_pretty(&up, pretty).expect("Fail");
    // writeln!(data_file, "{s}")?;

    upkreader::extract_by_name(&mut cursor, &up, path, dir_path, all)?;

    Ok(())
}

fn pack_upk(_ron_path: &str) -> Result<()> {
    unimplemented!("For now");
}

fn print_obj_elements(ron_path: &str, path: &str) -> Result<()> {
    if path.is_empty()
    {
        panic!("No object file provided");
    }

    if ron_path.is_empty()
    {
        panic!("No `.ron` file provided");
    }

    let ron_file = fs::read_to_string(ron_path)
        .unwrap_or_else(|_| panic!("File `{}` not found", ron_path));
    let ron_data: (String, String, UpkHeader, UPKPak) = ron::from_str(&ron_file).expect("RON Error");
    
    let upk: UPKPak = ron_data.3;
    let el_data = fs::read(path)?;
    let mut cursor = Cursor::new(&el_data);

    get_obj_props(&mut cursor, &upk, true)?;
    
    Ok(())
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
    #[command(about = "Print header info of upk file")]
    UpkHeader {
        path: String
    },

    #[command(about = "Print elements in object")]
    Elements {
        ron_path: String,
        path: String
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
        Commands::UpkHeader { path } => { upk_header_cursor(&path)?; },
        Commands::Elements { ron_path, path } => { 
            print_obj_elements(&ron_path, &path)?;
        },
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
