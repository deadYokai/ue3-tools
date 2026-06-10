use crate::upkreader::{UPKPak, UpkHeader, get_obj_props};
use clap::{Parser, Subcommand};
use std::{
    fs::{self, File},
    io::{BufReader, BufWriter, Cursor, Read, Result, Seek, SeekFrom, Write},
    path::Path,
};

use self::{
    types::font::{FontConfig, create_font_blobs, create_font_upk},
    utils::decompress::{CompressionMethod, upk_decompress},
};

mod native;
mod pseudo;
mod schema;
mod schemadb;
mod types;
mod upkpacker;
mod upkprops;
mod upkreader;
mod utils;
mod versions;

fn upk_header_cursor(path: &str) -> Result<(Cursor<Vec<u8>>, upkreader::UpkHeader)> {
    let path = Path::new(path);
    let file = File::open(path)?;
    let mut reader = BufReader::new(file);

    let filesize = reader.seek(SeekFrom::End(0))?;
    reader.seek(SeekFrom::Start(0))?;

    let header = UpkHeader::read(&mut reader)?;
    println!("{}", header);

    if header.compression_method == CompressionMethod::None || header.compressed_chunks_count == 0 {
        reader.seek(SeekFrom::Start(0))?;
        let mut buf = Vec::with_capacity(filesize as usize);
        reader.read_to_end(&mut buf)?;
        return Ok((Cursor::new(buf), header));
    }

    println!("File is compressed, decompressing in memory...");

    let mut cloned_header = header.clone();
    cloned_header.compression_method = CompressionMethod::None;
    cloned_header.compressed_chunks_count = 0;
    cloned_header.compressed_chunks.clear();
    cloned_header.pak_flags = header.pak_flags & !upkreader::PackageFlags::StoreCompressed.bits();

    let mut chunks = header.compressed_chunks.clone();
    chunks.sort_by_key(|c| c.decompressed_offset);

    let dec_data = upk_decompress(&mut reader, header.compression_method, &chunks)
        .expect("Decompression error");

    let dec_total = chunks
        .iter()
        .zip(dec_data.iter())
        .map(|(c, d)| c.decompressed_offset as usize + d.len())
        .max()
        .unwrap_or(0);

    let mut buf: Vec<u8> = Vec::with_capacity(dec_total.max(filesize as usize));
    {
        let mut w = std::io::Cursor::new(&mut buf);
        cloned_header.write(&mut w)?;
    }

    for (i, dec) in dec_data.iter().enumerate() {
        if i != 0 {
            let prev = chunks[i - 1].compressed_offset + chunks[i - 1].compressed_size;
            let gap = chunks[i].compressed_offset.saturating_sub(prev);
            if gap > 0 {
                reader.seek(SeekFrom::Start(prev as u64))?;
                let mut gap_buf = vec![0u8; gap as usize];
                reader.read_exact(&mut gap_buf)?;
                buf.extend_from_slice(&gap_buf);
            }
        }
        let target = chunks[i].decompressed_offset as usize;
        if buf.len() < target {
            buf.resize(target, 0);
        } else if buf.len() > target {
            buf[target..target + dec.len()].copy_from_slice(dec);
            continue;
        }
        buf.extend_from_slice(dec);
    }

    let last_compressed_end = chunks
        .last()
        .map(|c| (c.compressed_offset + c.compressed_size) as u64)
        .unwrap_or(0);
    if filesize > last_compressed_end {
        reader.seek(SeekFrom::Start(last_compressed_end))?;
        let mut tail = Vec::with_capacity((filesize - last_compressed_end) as usize);
        reader.read_to_end(&mut tail)?;
        buf.extend_from_slice(&tail);
    }

    Ok((Cursor::new(buf), cloned_header))
}

fn getlist(path: &str) -> Result<()> {
    let (cursor, header): (Cursor<Vec<u8>>, upkreader::UpkHeader) = upk_header_cursor(path)?;
    let mut cur: Cursor<&Vec<u8>> = Cursor::new(cursor.get_ref());

    let pak = UPKPak::parse_upk(&mut cur, &header)?;
    let list = upkreader::list_full_obj_paths(&pak);
    for (i, path) in list.iter().enumerate() {
        println!("#{} {}", i, path);
    }

    Ok(())
}

fn dump_names(upk_path: &str, mut output_path: &str) -> Result<()> {
    if output_path.is_empty() {
        output_path = "names_table.txt";
    }

    let (cursor, header): (Cursor<Vec<u8>>, upkreader::UpkHeader) = upk_header_cursor(upk_path)?;
    let mut cur: Cursor<&Vec<u8>> = Cursor::new(cursor.get_ref());
    cur.seek(SeekFrom::Start(header.name_offset as u64))?;

    println!("Names: (count = {})", header.name_count);

    let nt_file = File::create(Path::new(output_path))?;
    let mut writer = BufWriter::new(nt_file);

    for i in 0..header.name_count {
        let s = upkreader::read_name(&mut cur)?;
        println!("Name[{}]: {}", i, s.name);
        writeln!(writer, "{}", s.name)?;
    }

    Ok(())
}

fn extract_file(
    upk_path: &str,
    path: &str,
    mut output_dir: &str,
    all: bool,
    game_root: Option<&str>,
    verbose: bool,
) -> Result<()> {
    if output_dir.is_empty() {
        output_dir = "output";
    }

    let output_dir_path = Path::new(output_dir);

    let filename = Path::new(upk_path).file_stem().unwrap();

    let pbuf = output_dir_path.join(filename);
    let dir_path: &Path = pbuf.as_path();

    let (mut cursor, header) = upk_header_cursor(upk_path)?;
    let mut cur = Cursor::new(cursor.get_ref());
    let up = UPKPak::parse_upk(&mut cur, &header)?;

    if !dir_path.exists() {
        std::fs::create_dir_all(dir_path)?;
    }

    let db = match game_root {
        Some(gr) if !gr.is_empty() => {
            let db = schemadb::SchemaDb::new(Path::new(gr))?.with_verbose(verbose);
            let stem_lc = filename.to_string_lossy().to_lowercase();
            let lp = std::rc::Rc::new(schemadb::LazyPackage {
                stem_lc: stem_lc.clone(),
                path: Path::new(upk_path).to_path_buf(),
                bytes: cursor.get_ref().clone(),
                header: header.clone(),
                pak: up.clone(),
            });
            db.inject_package(lp);
            Some(db)
        }
        _ => None,
    };

    let stem_lc = filename.to_string_lossy().to_lowercase();
    upkreader::extract_by_name(
        &mut cursor,
        &up,
        path,
        dir_path,
        all,
        header.p_ver,
        db.as_ref(),
        &stem_lc,
    )?;
    Ok(())
}

fn pack_upk(_ron_path: &str) -> Result<()> {
    unimplemented!("For now");
}

fn print_obj_elements(ron_path: &str, path: &str) -> Result<()> {
    if path.is_empty() {
        panic!("No object file provided");
    }

    if ron_path.is_empty() {
        panic!("No `.ron` file provided");
    }

    let ron_file =
        fs::read_to_string(ron_path).unwrap_or_else(|_| panic!("File `{}` not found", ron_path));
    let ron_data: (String, String, UpkHeader, UPKPak) =
        ron::from_str(&ron_file).expect("RON Error");

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
    #[arg(long, global = true)]
    game_root: Option<String>,
    #[arg(short, long, global = true)]
    verbose: bool,
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    #[command(about = "Print header info of upk file")]
    UpkHeader {
        path: String,
    },

    #[command(about = "Print elements in object")]
    Elements {
        ron_path: String,
        path: String,
    },

    #[command(about = "Print list of objects in upk file")]
    List {
        path: String,
    },

    #[command(about = "Print or extract names in upk file")]
    Names {
        path: String,
        output_path: Option<String>,
    },

    #[command(about = "Extract specific object from upk")]
    Extract {
        upk_path: String,
        path: Option<String>,
        output_dir: Option<String>,
    },

    Pack {
        ron_path: String,
    },

    #[command(about = "Create a UE3 Font UPK from a TrueType / OpenType font file")]
    CreateFont {
        font_file: String,

        font_name: String,

        #[arg(long, default_value_t = 16.0)]
        size: f32,

        #[arg(long, default_value_t = 72)]
        dpi: u32,

        #[arg(long, default_value_t = 512)]
        tex_width: u32,

        #[arg(long, default_value_t = 512)]
        tex_height: u32,

        #[arg(long, default_value_t = 1)]
        x_pad: i32,

        #[arg(long, default_value_t = 1)]
        y_pad: i32,

        #[arg(long)]
        chars: Option<String>,

        #[arg(long)]
        upk: bool,

        #[arg(long, default_value_t = 684)]
        upk_version: i16,

        output_dir: Option<String>,
    },

    #[command(about = "Dump the meta-object schema for every export in a UPK")]
    SchemaDump {
        upk_path: String,
        #[arg(long)]
        class_filter: Option<String>,
    },

    #[command(about = "resolve a full path through the schema DB")]
    SchemaResolve {
        starting_pkg: String,
        full_path: String,
    },

    #[command(about = "open UI")]
    Ui,
}

fn schema_resolve(starting: &str, full_path: &str, game_root: &str, verbose: bool) -> Result<()> {
    use crate::schemadb::SchemaDb;
    use std::path::Path;

    let db = SchemaDb::new(Path::new(game_root))?.with_verbose(verbose);
    println!(
        "Indexed {} package(s), {} TFC(s) under {}",
        db.known_package_count(),
        db.tfc_index.len(),
        game_root
    );

    let r = db.resolve_full_path(starting, full_path)?;
    let r = match r {
        Some(r) => r,
        None => {
            println!("Resolution failed:");
            for m in db.misses.borrow().iter() {
                println!("  {m}");
            }
            return Ok(());
        }
    };
    println!("\nResolved: {}", r.display());
    let entry = db.entry(&r)?;
    println!("  entry: {}", summarize_entry(&entry));

    println!("\nClass chain:");
    let chain = db.class_chain(&r)?;
    for (i, link) in chain.iter().enumerate() {
        let name = db.export_object_name(link).unwrap_or_else(|| "?".into());
        println!("  {:2}. {}  ({})", i, name, link.display());
    }

    println!("\nDirect children:");
    for (name, cref, entry) in db.list_children(&r)? {
        println!(
            "  {:24}  {}  ({})",
            name,
            summarize_entry(&entry),
            cref.display()
        );
    }
    Ok(())
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::UpkHeader { path } => {
            upk_header_cursor(&path)?;
        }
        Commands::Elements { ron_path, path } => {
            print_obj_elements(&ron_path, &path)?;
        }
        Commands::List { path } => getlist(&path)?,
        Commands::Names { path, output_path } => {
            let out = output_path.as_deref().unwrap_or("");
            dump_names(&path, out)?
        }
        Commands::Extract {
            upk_path,
            path,
            output_dir,
        } => {
            let out = output_dir.as_deref().unwrap_or("");
            let mut extract_all = true;
            if path.is_some() {
                extract_all = false;
            }
            extract_file(
                &upk_path,
                path.as_deref().unwrap_or(""),
                out,
                extract_all,
                cli.game_root.as_deref(),
                cli.verbose,
            )?
        }
        Commands::Pack { .. } => unimplemented!(),
        Commands::CreateFont {
            font_file,
            font_name,
            size,
            dpi,
            tex_width,
            tex_height,
            x_pad,
            y_pad,
            chars,
            upk,
            upk_version,
            output_dir,
        } => {
            let out_dir = output_dir.as_deref().unwrap_or("output");
            create_font_cmd(
                &font_file,
                &font_name,
                size,
                dpi,
                tex_width,
                tex_height,
                x_pad,
                y_pad,
                chars.as_deref(),
                upk,
                upk_version,
                out_dir,
            )?;
        }

        Commands::SchemaDump {
            upk_path,
            class_filter,
        } => {
            schema_dump(&upk_path, class_filter.as_deref())?;
        }
        Commands::SchemaResolve {
            starting_pkg,
            full_path,
        } => {
            let gr = cli.game_root.as_deref().unwrap_or("");
            if gr.is_empty() {
                eprintln!("--game-root required for schema-resolve");
                std::process::exit(1);
            }
            schema_resolve(&starting_pkg, &full_path, gr, cli.verbose)?;
        }
        Commands::Ui => open_ui()?,
    }

    Ok(())
}

fn open_ui() -> Result<()> {
    Ok(())
}

fn schema_dump(upk_path: &str, class_filter: Option<&str>) -> Result<()> {
    use crate::schema::{SchemaParseCtx, parse_export_schema};

    let (mut cursor, header) = upk_header_cursor(upk_path)?;
    let mut cur = Cursor::new(cursor.get_ref());
    let pak = UPKPak::parse_upk(&mut cur, &header)?;

    let ctx = SchemaParseCtx {
        p_ver: header.p_ver,
        cooked_for_console: false,
    };

    println!("Parsing schema (p_ver={})", ctx.p_ver);

    let mut ok = 0usize;
    let mut skipped = 0usize;
    let mut failed = 0usize;

    for (idx, exp) in pak.export_table.iter().enumerate() {
        let class_name = pak.get_class_name(exp.class_index);
        if let Some(f) = class_filter {
            if !class_name.contains(f) {
                continue;
            }
        }

        cursor.seek(SeekFrom::Start(exp.serial_offset as u64))?;
        let mut blob = vec![0u8; exp.serial_size as usize];
        cursor.read_exact(&mut blob)?;

        let full_name = pak.get_export_full_name((idx + 1) as i32);
        match parse_export_schema(&blob, &class_name, &pak, ctx) {
            Ok(Some(entry)) => {
                ok += 1;
                println!(
                    "#{:5} {}  →  {}",
                    idx + 1,
                    full_name,
                    summarize_entry(&entry)
                );
            }
            Ok(None) => {
                skipped += 1;
            }
            Err(e) => {
                failed += 1;
                eprintln!("#{:5} {}  →  parse error: {}", idx + 1, full_name, e);
            }
        }
    }

    println!(
        "\nSummary: {} parsed, {} skipped (non-meta), {} failed",
        ok, skipped, failed
    );
    Ok(())
}

fn summarize_entry(e: &crate::schema::SchemaEntry) -> String {
    use crate::schema::SchemaEntry::*;
    match e {
        Struct { header } => format!(
            "Struct super={} children=0x{:x}",
            header.super_struct, header.children
        ),
        Function { header, extra } => format!(
            "Function super={} children=0x{:x} flags=0x{:08x} iNative={}",
            header.super_struct, header.children, extra.function_flags, extra.i_native
        ),
        State { header, extra } => format!(
            "State super={} children=0x{:x} state_flags=0x{:08x} funcs={}",
            header.super_struct,
            header.children,
            extra.state_flags,
            extra.func_map.len()
        ),
        Class { header, extra, .. } => format!(
            "Class super={} children=0x{:x} class_flags=0x{:08x} CDO=#{} ifaces={}",
            header.super_struct,
            header.children,
            extra.class_flags,
            extra.class_default_object,
            extra.interfaces.len()
        ),
        ScriptStruct { header, extra } => format!(
            "ScriptStruct super={} children=0x{:x} struct_flags=0x{:08x}",
            header.super_struct, header.children, extra.struct_flags
        ),
        Enum { names, .. } => format!("Enum [{}]", names.len()),
        Const { value, .. } => format!("Const = {value:?}"),
        Property(p) => {
            let c = p.common();
            format!(
                "{:?} dim={} flags=0x{:016x}",
                std::mem::discriminant(p),
                c.array_dim,
                c.property_flags
            )
        }
        OpaqueChild { class_name, next } => {
            format!("OpaqueChild({class_name}) next={next}")
        }
    }
}
fn create_font_cmd(
    font_file: &str,
    font_name: &str,
    size: f32,
    dpi: u32,
    tex_width: u32,
    tex_height: u32,
    x_pad: i32,
    y_pad: i32,
    chars: Option<&str>,
    write_upk: bool,
    upk_version: i16,
    out_dir: &str,
) -> std::io::Result<()> {
    let cfg = FontConfig {
        font_path: font_file.to_string(),
        font_name: font_name.to_string(),
        size_pt: size,
        dpi,
        tex_width,
        tex_height,
        x_pad,
        y_pad,
        chars: chars.map(|s| s.to_string()),
        upk_version,
    };

    std::fs::create_dir_all(out_dir)?;
    create_font_blobs(&cfg, Path::new(out_dir))?;

    if write_upk {
        let out_path = Path::new(out_dir).join(format!("{}.upk", font_name));
        create_font_upk(&cfg, &out_path)?;
    }

    Ok(())
}
