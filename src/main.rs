use crate::{
    upkdecompress::{CompressionMethod, upk_decompress},
    upkreader::{PackageFlags, UPKPak, UpkHeader, get_obj_props},
};
use clap::{Parser, Subcommand};
use ron::ser::{PrettyConfig, to_string_pretty};
use std::{
    fs::{self, File},
    io::{BufReader, BufWriter, Cursor, Read, Result, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
};

use self::upkfont::create_font_blobs;

mod mod_engine;
mod schema;
mod schemadb;
mod scriptcompiler;
mod scriptdisasm;
mod scriptpatcher;
mod upkdecompress;
mod upkfont;
mod upkpacker;
mod upkprops;
mod upkreader;
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

fn make_script_patch(
    package_name: &str,
    struct_name: &str,
    function_path: &str,
    bytecode_file: &str,
    output_dir: &str,
) -> Result<()> {
    use crate::scriptpatcher::{LinkerPatchData, ScriptPatchData, compress_patch};

    let bytecode = fs::read(bytecode_file)?;
    let mut patch = LinkerPatchData::new(package_name.to_string());
    patch.add_script_patch(ScriptPatchData::new(
        struct_name.to_string(),
        function_path.to_string(),
        bytecode,
    ));
    let (bin, unc) = compress_patch(&patch)?;
    fs::create_dir_all(output_dir)?;
    let out = Path::new(output_dir).join(format!("ScriptPatch_{}.bin", package_name));
    fs::write(&out, &bin)?;
    fs::write(
        format!("{}.uncompressed_size", out.display()),
        format!("{}", unc),
    )?;
    println!("Wrote patch: {}", out.display());
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
            eprintln!(
                "No export matching '{}' found in {}",
                function_path, upk_path
            );
            return Ok(());
        }
    };

    let exp = &pak.export_table[exp_idx];
    let full_name = pak.get_export_full_name((exp_idx + 1) as i32);
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
    let script = scriptdisasm::extract_script_from_export_blob(&blob, &pak).unwrap_or_else(|| {
        eprintln!("warn: could not locate Script array; disassembling raw blob");
        blob.clone()
    });

    let stmts = scriptdisasm::disasm_function(&script, &pak);
    let text = scriptdisasm::print_disasm(&stmts);
    println!("{}", text);

    // Write .asm file.
    let out_dir = Path::new(output_dir);
    let upk_stem = Path::new(upk_path).file_stem().unwrap().to_str().unwrap();
    let fn_name = function_path
        .rsplit(['/', '.'])
        .next()
        .unwrap_or(function_path);
    let dir_path = out_dir.join(upk_stem);
    std::fs::create_dir_all(&dir_path)?;
    let asm_path = dir_path.join(format!("{}.asm", fn_name));
    std::fs::write(&asm_path, &text)?;
    println!("Wrote: {}", asm_path.display());

    Ok(())
}

fn compile_asm(upk_path: &str, asm_file: &str, output_file: &str) -> Result<()> {
    let text = fs::read_to_string(asm_file)?;
    let (cursor, header) = upk_header_cursor(upk_path)?;
    let mut cur = Cursor::new(cursor.get_ref());
    let pak = UPKPak::parse_upk(&mut cur, &header)?;
    let mut compiler = scriptcompiler::Compiler::new(&pak);
    let bytecode = compiler.compile_text(&text)?;
    fs::write(output_file, &bytecode)?;
    println!("Compiled {} bytes → {}", bytecode.len(), output_file);
    Ok(())
}

fn make_object_patch(
    package_name: &str,
    object_path: &str, // e.g. "DishonoredGame.DishWeaponSword"
    data_file: &str,   // raw serialized property data (tagged properties blob)
    output_dir: &str,
) -> Result<()> {
    use crate::scriptpatcher::{LinkerPatchData, PatchData, compress_patch};

    let data = fs::read(data_file)?;
    let mut patch = LinkerPatchData::new(package_name.to_string());
    patch.add_cdo_patch(PatchData::new(object_path.to_string(), data));
    let (bin, unc) = compress_patch(&patch)?;
    fs::create_dir_all(output_dir)?;
    let out = Path::new(output_dir).join(format!("ScriptPatch_{}.bin", package_name));
    fs::write(&out, &bin)?;
    fs::write(
        format!("{}.uncompressed_size", out.display()),
        format!("{}", unc),
    )?;
    println!("Wrote CDO patch: {}", out.display());
    Ok(())
}

// ── NEW: apply a .bin patch directly to a UPK file (offline) ────────────────
fn apply_patch_cmd(patch_file: &str, upk_path: &str, output_path: Option<&str>) -> Result<()> {
    use crate::scriptpatcher::{apply_patches_to_upk, load_patch_bin};

    let bin = fs::read(patch_file)?;
    let patch = load_patch_bin(&bin)?;

    println!("Patch: package={}", patch.package_name);
    println!("  {} script patch(es)", patch.script_patches.len());
    println!(
        "  {} CDO patch(es)",
        patch.modified_class_default_objects.len()
    );
    println!("  {} enum patch(es)", patch.modified_enums.len());
    println!("  {} new object(s)", patch.new_objects.len());

    let (cursor, header) = upk_header_cursor(upk_path)?;
    let upk_raw = cursor.into_inner();
    let mut cur: Cursor<&Vec<u8>> = Cursor::new(&upk_raw);
    let pak = UPKPak::parse_upk(&mut cur, &header)?;

    let patched = apply_patches_to_upk(&upk_raw, &header, &pak, &patch)?;

    let out = output_path.map(|s| s.to_string()).unwrap_or_else(|| {
        let stem = Path::new(upk_path).file_stem().unwrap().to_str().unwrap();
        format!("{}.patched.upk", stem)
    });
    fs::write(&out, &patched)?;
    println!("Wrote patched UPK: {}", out);
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
        path: String,
        output_dir: Option<String>,
    },

    #[command(about = "Extract all objects from upk")]
    Extractall {
        upk_path: String,
        output_dir: Option<String>,
    },

    Pack {
        ron_path: String,
    },

    #[command(about = "Create script patch bin")]
    MakeScriptPatch {
        package_name: String,
        struct_name: String,
        function_path: String,
        bytecode_file: String,
        output_dir: Option<String>,
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

    MakeObjectPatch {
        package_name: String,
        object_path: String,
        data_file: String,
        output_dir: Option<String>,
    },

    #[command(about = "Apply a script patch .bin directly to a UPK file")]
    ApplyPatch {
        patch_file: String,
        upk_path: String,
        output_path: Option<String>,
    },

    #[command(about = "Create a font CDO patch .bin targeting a font inside an existing UPK")]
    MakeFontPatch {
        upk_path: String,

        font_object_name: String,

        font_file: String,

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

        output_dir: Option<String>,
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
    #[command(about = "Create a new mod directory")]
    ModNew {
        name: String,
        #[arg(long, default_value = ".")]
        out: String,
    },

    #[command(about = "Extract a UPK export blob into a mod dir")]
    ModExtract {
        upk_path: String,
        obj_name: String,
        mod_dir: String,
        #[arg(long)]
        dir: String,
    },

    #[command(about = "Pack mod dir into ScriptPatch_*.bin files")]
    ModPack {
        mod_dir: String,
        #[arg(long, default_value = "dist")]
        out: String,
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
            extract_file(
                &upk_path,
                &path,
                out,
                false,
                cli.game_root.as_deref(),
                cli.verbose,
            )?
        }
        Commands::Extractall {
            upk_path,
            output_dir,
        } => {
            let out = output_dir.as_deref().unwrap_or("");
            extract_file(
                &upk_path,
                "",
                out,
                true,
                cli.game_root.as_deref(),
                cli.verbose,
            )?
        }
        Commands::Pack { .. } => unimplemented!(),
        Commands::MakeScriptPatch {
            package_name,
            struct_name,
            function_path,
            bytecode_file,
            output_dir,
        } => {
            let out = output_dir.as_deref().unwrap_or("patches");
            make_script_patch(
                &package_name,
                &struct_name,
                &function_path,
                &bytecode_file,
                out,
            )?;
        }
        Commands::Disasm {
            upk_path,
            function_path,
            output_dir,
        } => {
            let out = output_dir.as_deref().unwrap_or("output");
            disasm_function_cmd(&upk_path, &function_path, out)?;
        }
        Commands::Compile {
            upk_path,
            asm_file,
            output_file,
        } => {
            let out = output_file.as_deref().unwrap_or("output.bin");
            compile_asm(&upk_path, &asm_file, out)?;
        }
        Commands::MakeObjectPatch {
            package_name,
            object_path,
            data_file,
            output_dir,
        } => {
            let out = output_dir.as_deref().unwrap_or("patches");
            make_object_patch(&package_name, &object_path, &data_file, out)?;
        }
        Commands::ApplyPatch {
            patch_file,
            upk_path,
            output_path,
        } => {
            apply_patch_cmd(&patch_file, &upk_path, output_path.as_deref())?;
        }
        Commands::MakeFontPatch {
            upk_path,
            font_object_name,
            font_file,
            size,
            dpi,
            tex_width,
            tex_height,
            x_pad,
            y_pad,
            chars,
            output_dir,
        } => {
            let out = output_dir.as_deref().unwrap_or("patches");
            make_font_patch_cmd(
                &upk_path,
                &font_object_name,
                &font_file,
                size,
                dpi,
                tex_width,
                tex_height,
                x_pad,
                y_pad,
                chars.as_deref(),
                out,
            )?;
        }
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
        Commands::ModNew { name, out } => {
            mod_engine::cmd_new(&name, Path::new(&out))?;
        }
        Commands::ModExtract {
            upk_path,
            obj_name,
            mod_dir,
            dir,
        } => {
            mod_engine::cmd_extract(Path::new(&upk_path), &obj_name, Path::new(&mod_dir), &dir)?;
        }
        Commands::ModPack { mod_dir, out } => {
            let mod_path = Path::new(&mod_dir);
            let dist_path = if Path::new(&out).is_absolute() {
                PathBuf::from(&out)
            } else {
                mod_path.join(&out)
            };
            mod_engine::cmd_pack(mod_path, &dist_path)?;
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
    }

    Ok(())
}

fn schema_dump(upk_path: &str, class_filter: Option<&str>) -> Result<()> {
    use crate::schema::{SchemaParseCtx, parse_export_schema};

    let (mut cursor, header) = upk_header_cursor(upk_path)?;
    let mut cur = Cursor::new(cursor.get_ref());
    let pak = UPKPak::parse_upk(&mut cur, &header)?;

    let ctx = SchemaParseCtx {
        p_ver: header.p_ver,
        strip_editor_only: header.strip_editor_only(),
    };

    println!(
        "Parsing schema (p_ver={}, strip_editor_only={})",
        ctx.p_ver, ctx.strip_editor_only
    );

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
        Class {
            header,
            class_flags,
            class_default_object,
            ..
        } => format!(
            "Class super={} children=0x{:x} cdo={} flags=0x{:08x}",
            header.super_struct, header.children, class_default_object, class_flags
        ),
        State {
            header,
            state_flags,
            ..
        } => format!(
            "State super={} children=0x{:x} flags=0x{:08x}",
            header.super_struct, header.children, state_flags
        ),
        ScriptStruct {
            header,
            struct_flags,
        } => format!(
            "ScriptStruct super={} children=0x{:x} flags=0x{:08x}",
            header.super_struct, header.children, struct_flags
        ),
        Struct { header } => format!(
            "Struct super={} children=0x{:x}",
            header.super_struct, header.children
        ),
        Function {
            header,
            function_flags,
            i_native,
            ..
        } => format!(
            "Function super={} children=0x{:x} iNative={} flags=0x{:08x} script={}B",
            header.super_struct,
            header.children,
            i_native,
            function_flags,
            header.on_disk_script_size
        ),
        Enum { names, .. } => format!("Enum [{}]", names.len()),
        Property(p) => {
            let c = p.common();
            format!(
                "{:?} dim={} flags=0x{:016x}",
                std::mem::discriminant(p),
                c.array_dim,
                c.property_flags
            )
        }
    }
}

fn make_font_patch_cmd(
    upk_path: &str,
    font_object_name: &str,
    font_file: &str,
    size: f32,
    dpi: u32,
    tex_width: u32,
    tex_height: u32,
    x_pad: i32,
    y_pad: i32,
    chars: Option<&str>,
    out_dir: &str,
) -> Result<()> {
    use crate::upkfont::{FontConfig, create_font_patch};

    let (cursor, header) = upk_header_cursor(upk_path)?;
    let upk_raw = cursor.into_inner();
    let mut cur: Cursor<&Vec<u8>> = Cursor::new(&upk_raw);
    let pak = UPKPak::parse_upk(&mut cur, &header)?;

    let package_name = Path::new(upk_path)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("Unknown");

    let cfg = FontConfig {
        font_path: font_file.to_string(),
        font_name: font_object_name.to_string(),
        size_pt: size,
        dpi,
        tex_width,
        tex_height,
        x_pad,
        y_pad,
        chars: chars.map(|s| s.to_string()),
        upk_version: header.p_ver,
    };

    create_font_patch(
        &upk_raw,
        &header,
        &pak,
        font_object_name,
        &cfg,
        package_name,
        Path::new(out_dir),
    )
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
    use crate::upkfont::{FontConfig, create_font_upk};

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
