use std::{
    fs,
    io::{self, Cursor, Read, Seek, SeekFrom},
    path::{Path, PathBuf},
};

use serde::Deserialize;

use crate::{
    scriptpatcher::{LinkerPatchData, PatchData, compress_patch},
    upkreader::{UPKPak, UpkHeader},
};

#[derive(Debug, Deserialize)]
pub struct ModManifest {
    #[serde(rename = "mod")]
    pub meta:  ModMeta,
    #[serde(default)]
    pub patch: Vec<PatchBlock>,
}

#[derive(Debug, Deserialize)]
pub struct ModMeta {
    pub name:        String,
    pub version:     String,
    #[serde(default)]
    pub author:      String,
    #[serde(default)]
    pub description: String,
}

#[derive(Debug, Deserialize)]
pub struct PatchBlock {
    pub dir:     String,
    pub package: String,
    #[serde(default)]
    pub replace: Vec<ReplaceEntry>,
}

#[derive(Debug, Deserialize)]
pub struct ReplaceEntry {
    pub original: String,
    pub modfile:  String,
}

pub fn cmd_new(name: &str, out: &Path) -> io::Result<()> {
    let mod_dir = out.join(name);
    fs::create_dir_all(mod_dir.join("dist"))?;

    let toml = format!(
        r#"[mod]
name        = "{name}"
version     = "0.1"
author      = ""
description = ""

# Add one [[patch]] block per UPK you want to replace exports in.
# Then run:
#   ue3-tools mod extract <Pkg.upk> <ExportName> . --dir <subdir>
# to populate the blob files.

# [[patch]]
# dir     = "Fonts"
# package = "UI_Loading"
#
#   [[patch.replace]]
#   original = "Emerge_BF"
#   modfile  = "MyHudFont"
"#
    );

    let toml_path = mod_dir.join("mod.toml");
    if toml_path.exists() {
        eprintln!("mod.toml already exists at {}", toml_path.display());
    } else {
        fs::write(&toml_path, &toml)?;
    }

    println!("✓  {}", mod_dir.display());
    println!("   edit mod.toml, then:");
    println!("   ue3-tools mod extract <Pkg.upk> <ObjName> {} --dir <subdir>", mod_dir.display());
    println!("   ue3-tools mod pack {}", mod_dir.display());
    Ok(())
}

pub fn cmd_extract(
    upk_path: &Path,
    obj_name: &str,
    mod_dir:  &Path,
    subdir:   &str,
) -> io::Result<()> {
    let raw     = fs::read(upk_path)?;
    let mut cur = Cursor::new(raw.as_slice());
    let header  = UpkHeader::read(&mut cur)?;

    cur.seek(SeekFrom::Start(0))?;
    let raw_vec: Vec<u8> = raw.clone();
    let mut cv  = Cursor::new(&raw_vec);
    let pak     = UPKPak::parse_upk(&mut cv, &header)?;

    let needle = obj_name.to_lowercase();
    let found  = pak.export_table.iter().enumerate().find(|(i, _)| {
        let full = pak.get_export_full_name((*i + 1) as i32);
        let path = pak.get_export_path_name((*i + 1) as i32);
        path.to_lowercase().contains(&needle)
            || full.to_lowercase().contains(&needle)
    });

    let (exp_idx, exp) = found.ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            format!("no export matching '{}' in {}", obj_name, upk_path.display()),
        )
    })?;

    let class_name = pak.get_class_name(exp.class_index);
    let path_name  = pak.get_export_path_name((exp_idx + 1) as i32);
    let stem = path_name.rsplit('.').next().unwrap_or(obj_name);

    let out_dir = mod_dir.join(subdir);
    fs::create_dir_all(&out_dir)?;

    let blob_path = out_dir.join(format!("{}.{}", stem, class_name));

    let s = exp.serial_offset as usize;
    let e = s + exp.serial_size as usize;
    if e > raw.len() {
        return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "serial data out of bounds"));
    }
    fs::write(&blob_path, &raw[s..e])?;

    println!(
        "✓  {} bytes → {}",
        exp.serial_size,
        blob_path.display()
    );
    println!("   add to mod.toml:");
    println!("     [[patch.replace]]");
    println!("     original = \"{}\"", stem);
    println!("     modfile  = \"{}\"", stem);
    Ok(())
}

pub fn cmd_pack(mod_dir: &Path, dist_dir: &Path) -> io::Result<()> {
    let manifest = load_manifest(mod_dir)?;
    fs::create_dir_all(dist_dir)?;

    for block in &manifest.patch {
        let bin = build_patch_bin(mod_dir, block)?;
        let out = dist_dir.join(format!("ScriptPatch_{}.bin", block.package));
        fs::write(&out, &bin)?;
        println!(
            "✓  {} replace(s) → {}",
            block.replace.len(),
            out.display()
        );
    }

    println!("pack done — copy dist/ contents to Mods/ next to the game EXE.");
    Ok(())
}


pub fn load_manifest(mod_dir: &Path) -> io::Result<ModManifest> {
    let toml_path = mod_dir.join("mod.toml");
    let text = fs::read_to_string(&toml_path).map_err(|e| {
        io::Error::new(e.kind(), format!("{}: {e}", toml_path.display()))
    })?;
    toml::from_str(&text).map_err(|e| {
        io::Error::new(io::ErrorKind::InvalidData, format!("mod.toml parse error: {e}"))
    })
}

fn build_patch_bin(mod_dir: &Path, block: &PatchBlock) -> io::Result<Vec<u8>> {
    let mut lpd = LinkerPatchData::new(block.package.clone());

    for rep in &block.replace {
        let blob = find_blob(mod_dir, &block.dir, &rep.modfile)?;
        lpd.add_cdo_patch(PatchData::new(rep.original.clone(), blob));
    }

    compress_patch(&lpd).map_err(|e| {
        io::Error::new(io::ErrorKind::Other, format!("compress_patch: {e}"))
    })
}

fn find_blob(mod_dir: &Path, subdir: &str, stem: &str) -> io::Result<Vec<u8>> {
    let dir = mod_dir.join(subdir);
    for entry in fs::read_dir(&dir).map_err(|e| {
        io::Error::new(e.kind(), format!("{}: {e}", dir.display()))
    })? {
        let path = entry?.path();
        if path.file_stem().and_then(|s| s.to_str()) == Some(stem) {
            return fs::read(&path);
        }
    }
    Err(io::Error::new(
        io::ErrorKind::NotFound,
        format!(
            "blob not found: {}/{}/{}.* — run `mod extract` first",
            subdir,
            stem,
            mod_dir.display()
        ),
    ))
}
