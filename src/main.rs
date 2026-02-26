use std::{fs::{self, File}, io::{BufReader, BufWriter, Cursor, Read, Result, Seek, SeekFrom, Write}, path::Path};
use ron::{ser::{to_string_pretty, PrettyConfig}};
use crate::{upkdecompress::{upk_decompress, CompressionMethod}, upkreader::{get_obj_props, PackageFlags, UPKPak, UpkHeader}};
use clap::{Parser, Subcommand};

mod upkreader;
mod upkpacker;
mod upkdecompress;
mod upkprops;
mod upkfont;
mod scriptpatcher;
mod scriptcompiler;
mod scriptdisasm;

fn upk_header_cursor(path: &str) -> Result<(Cursor<Vec<u8>>, upkreader::UpkHeader)>
{
    let path = Path::new(path);
    let file = File::open(path)?;
    let mut reader = BufReader::new(file);

    let filesize = reader.seek(SeekFrom::End(0))?;
    reader.seek(SeekFrom::Start(0))?;

    let header = UpkHeader::read(&mut reader)?;
    println!("{}", header);

    let end_header_offest = reader.stream_position()? as usize;
     
    if header.compression_method != CompressionMethod::None 
    {

        if header.compressed_chunks_count != 0 {

            println!("File is compressed, trying decompress...");

            let mut cloned_header = header.clone();
            cloned_header.compression_method = CompressionMethod::None;
            cloned_header.compressed_chunks_count = 0;
            cloned_header.pak_flags = header.pak_flags & !PackageFlags::StoreCompressed.bits();

            let mut chunks = header.compressed_chunks;

            chunks.sort_by_key(|c| c.decompressed_offset);

            let first_chunk_offset = chunks[0].compressed_offset as usize;

            let dec_data = upk_decompress(&mut reader, header.compression_method, &chunks)
                .expect("Decompression error"); 

            let file = File::create(".tmp.upk")?;
            let mut writer = BufWriter::new(file);

            cloned_header.write(&mut writer)?;

            //
            // println!("{:?} {:?}", first_chunk_offset, end_header_offest);
            // // TODO: hmm, 4 zerobytes, need find what it is
            //
            // if first_chunk_offset > end_header_offest {
            //     let pre_data_len = first_chunk_offset - end_header_offest - (chunks.len() * 16);
            //     reader.seek(SeekFrom::Start((end_header_offest + (chunks.len() * 16)) as u64))?;
            //     let mut pre_data = vec![0u8; pre_data_len];
            //     reader.read_exact(&mut pre_data)?;
            //     writer.write_all(&pre_data)?;
            // }
            // 
            for (i, c) in dec_data.iter().enumerate() {
                if i != 0 {
                    let prev = chunks[i-1].compressed_offset +
                        chunks[i-1].compressed_size;

                    let diff = chunks[i].compressed_offset - prev;

                    if diff > 0 {
                        reader.seek(SeekFrom::Start(prev as u64))?;
                        let mut data = vec![0u8; diff as usize];
                        reader.read_exact(&mut data)?;
                        writer.write_all(&data)?;
                    }
                }
                writer.seek(SeekFrom::Start(chunks[i].decompressed_offset as u64))?;
                writer.write_all(c)?;
            }

            let last = chunks[chunks.len() - 1].compressed_offset +
                chunks[chunks.len() - 1].compressed_size;

            if filesize > last as u64 {
                 reader.seek(SeekFrom::Start(last as u64))?;
                 let mut data = vec![0u8; (filesize - last as u64) as usize];
                 reader.read_exact(&mut data)?;
                 writer.write_all(&data)?;
            }
 
        }

        println!("File is decompressed. Reopening file");

        fs::remove_file(path)?;
        fs::rename(".tmp.upk", path)?;
        return upk_header_cursor(path.to_str().unwrap());
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

    let pak = UPKPak::parse_upk(&mut cur, &header)?;
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
    let up = UPKPak::parse_upk(&mut cur, &header)?;

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

    upkreader::extract_by_name(&mut cursor, &up, path, dir_path, all, header.p_ver)?;

    Ok(())
}

fn make_script_patch(
    package_name: &str,
    struct_name: &str,
    function_path: &str,
    bytecode_file: &str,
    output_dir: &str,
) -> Result<()> {
    use crate::scriptpatcher::{LinkerPatchData, ScriptPatchData};
    use flate2::write::ZlibEncoder;
    use flate2::Compression;

    let bytecode = fs::read(bytecode_file)?;

    let mut patch = LinkerPatchData::new(package_name.to_string());
    patch.add_script_patch(ScriptPatchData::new(
        struct_name.to_string(),
        function_path.to_string(),
        bytecode,
    ));

    let mut uncompressed: Vec<u8> = Vec::new();
    patch.serialize(&mut uncompressed)?;

    let block_size: usize = 0x20000;
    let uncompressed_total = uncompressed.len() as u32;
    let blocks: Vec<Vec<u8>> = uncompressed
        .chunks(block_size)
        .map(|chunk| {
            let mut enc = ZlibEncoder::new(Vec::new(), Compression::default());
            enc.write_all(chunk).unwrap();
            enc.finish().unwrap()
        })
        .collect();

    let compressed_total: u32 = blocks.iter().map(|b| b.len() as u32).sum();

    let mut out_buf: Vec<u8> = Vec::new();
    out_buf.write_all(&uncompressed_total.to_le_bytes())?;
    out_buf.write_all(&compressed_total.to_le_bytes())?;
    for (i, block) in blocks.iter().enumerate() {
        let unc_size = if i == blocks.len() - 1 {
            uncompressed.len() - i * block_size
        } else {
            block_size
        };
        out_buf.write_all(&(block.len() as u32).to_le_bytes())?;
        out_buf.write_all(&(unc_size as u32).to_le_bytes())?;
    }
    for block in &blocks {
        out_buf.write_all(block)?;
    }

    let out_path = Path::new(output_dir);
    fs::create_dir_all(out_path)?;
    let bin_path = out_path.join(format!("ScriptPatch_{}.bin", package_name));
    fs::write(&bin_path, &out_buf)?;
    println!("Wrote patch: {}", bin_path.display());

    Ok(())
}

fn disasm_function_cmd(upk_path: &str, function_path: &str, output_dir: &str) -> Result<()> {
    let (mut cursor, header) = upk_header_cursor(upk_path)?;
    let mut cur = Cursor::new(cursor.get_ref());
    let pak = UPKPak::parse_upk(&mut cur, &header)?;

    // Find the export whose full name matches function_path.
    let needle = function_path.to_lowercase();
    let export_entry = pak.export_table.iter().enumerate().find(|(idx, _)| {
        let full = pak.get_export_full_name((*idx + 1) as i32);
        full.to_lowercase().contains(&needle)
    });

    let (exp_idx, _) = match export_entry {
        Some(e) => e,
        None => {
            eprintln!("No export matching '{}' found in {}", function_path, upk_path);
            return Ok(());
        }
    };

    let exp        = &pak.export_table[exp_idx];
    let full_name  = pak.get_export_full_name((exp_idx + 1) as i32);
    let class_name = pak.get_class_name(exp.class_index);

    if class_name != "Function" && class_name != "ScriptFunction" {
        eprintln!(
            "Export '{}' has class '{}', not a script function.",
            full_name, class_name
        );
        return Ok(());
    }

    // Read the raw serial blob.
    cursor.seek(SeekFrom::Start(exp.serial_offset as u64))?;
    let mut blob = vec![0u8; exp.serial_size as usize];
    cursor.read_exact(&mut blob)?;

    // Extract and disassemble the Script TArray.
    let script = scriptdisasm::extract_script_from_export_blob(&blob, &pak)
        .unwrap_or_else(|| {
            eprintln!("warn: could not locate Script array; disassembling raw blob");
            blob.clone()
        });

    let stmts = scriptdisasm::disasm_function(&script, &pak);
    let text  = scriptdisasm::print_disasm(&stmts);
    println!("{}", text);

    // Write .asm file.
    let out_dir = Path::new(output_dir);
    let upk_stem = Path::new(upk_path).file_stem().unwrap().to_str().unwrap();
    let fn_name  = function_path.rsplit(['/', '.']).next().unwrap_or(function_path);
    let dir_path = out_dir.join(upk_stem);
    std::fs::create_dir_all(&dir_path)?;
    let asm_path = dir_path.join(format!("{}.asm", fn_name));
    std::fs::write(&asm_path, &text)?;
    println!("Wrote: {}", asm_path.display());

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
    let header: UpkHeader = ron_data.2;
    let el_data = fs::read(path)?;
    let mut cursor = Cursor::new(&el_data);

    let (_, _) = get_obj_props(&mut cursor, &upk, true, header.p_ver)?;
    
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
    },

    #[command(about = "Create script patch bin")]
    MakeScriptPatch {
        package_name: String,
        struct_name: String,
        function_path: String,
        bytecode_file: String,
        output_dir: Option<String>
    },

    #[command(about = "Disassemble UnrealScript bytecode from a UPK function")]
    Disasm {
        upk_path: String,
        /// Full object path, e.g. "MyPackage.MyClass.MyFunction"
        function_path: String,
        output_dir: Option<String>,
    },

    /// Compile an assembly text file to raw UnrealScript bytecode.
    #[command(about = "Compile bytecode assembly text to .bin for MakeScriptPatch")]
    Compile {
        upk_path: String,
        asm_file: String,
        output_file: Option<String>,
    },
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
        Commands::Pack { .. } => unimplemented!(),
        Commands::MakeScriptPatch
            { package_name, struct_name, function_path, bytecode_file, output_dir } => 
        {
            let out = output_dir.as_deref().unwrap_or("Patches");
            make_script_patch
                (&package_name, &struct_name, &function_path, &bytecode_file, out)?;
        },
        Commands::Disasm { upk_path, function_path, output_dir } => {
            let out = output_dir.as_deref().unwrap_or("output");
            disasm_function_cmd(&upk_path, &function_path, out)?;
        }
        Commands::Compile { upk_path, asm_file, output_file } => {
            unimplemented!()
        }

    }

    Ok(())
}
