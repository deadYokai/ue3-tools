use crate::{
    scriptdisasm::extract_script_from_export_blob,
    upkreader::{UPKPak, UpkHeader},
};
use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};
use flate2::{Compression, read::ZlibDecoder, write::ZlibEncoder};
use std::{
    collections::HashMap,
    io::{self, Read, Write},
};

pub const PACKAGE_FILE_TAG: u32 = 0x9E2A83C1;

pub const COMPRESS_ZLIB: i32 = 1;

pub const CHUNK_SIZE: usize = 0x20000;

pub fn write_ue3_string<W: Write>(w: &mut W, s: &str) -> io::Result<()> {
    if s.is_empty() {
        return w.write_i32::<LittleEndian>(0);
    }
    let b = s.as_bytes();
    w.write_i32::<LittleEndian>((b.len() as i32) + 1)?;
    w.write_all(b)?;
    w.write_u8(0)
}

pub fn read_ue3_string<R: Read>(r: &mut R) -> io::Result<String> {
    let len = r.read_i32::<LittleEndian>()?;
    if len == 0 {
        return Ok(String::new());
    }
    if len > 0 {
        let mut b = vec![0u8; len as usize];
        r.read_exact(&mut b)?;
        if b.last() == Some(&0) {
            b.pop();
        }
        Ok(String::from_utf8_lossy(&b).into_owned())
    } else {
        let count = (-len) as usize;
        let mut chars = Vec::with_capacity(count);
        for _ in 0..count {
            chars.push(r.read_u16::<LittleEndian>()?);
        }
        if chars.last() == Some(&0) {
            chars.pop();
        }
        Ok(String::from_utf16_lossy(&chars))
    }
}

fn write_byte_array<W: Write>(w: &mut W, data: &[u8]) -> io::Result<()> {
    w.write_i32::<LittleEndian>(data.len() as i32)?;
    w.write_all(data)
}

fn read_byte_array<R: Read>(r: &mut R) -> io::Result<Vec<u8>> {
    let n = r.read_i32::<LittleEndian>()?;
    if n <= 0 {
        return Ok(Vec::new());
    }
    let mut b = vec![0u8; n as usize];
    r.read_exact(&mut b)?;
    Ok(b)
}

#[derive(Debug, Clone)]
pub struct PatchData {
    pub data_name: String,
    pub data: Vec<u8>,
}

impl PatchData {
    pub fn new(data_name: String, data: Vec<u8>) -> Self {
        Self { data_name, data }
    }

    pub fn serialize<W: Write>(&self, w: &mut W) -> io::Result<()> {
        write_ue3_string(w, &self.data_name)?;
        write_byte_array(w, &self.data)
    }

    pub fn deserialize<R: Read>(r: &mut R) -> io::Result<Self> {
        Ok(Self {
            data_name: read_ue3_string(r)?,
            data: read_byte_array(r)?,
        })
    }
}

#[derive(Debug, Clone)]
pub struct ScriptPatchData {
    pub struct_name: String,
    pub patch_data: PatchData,
}

impl ScriptPatchData {
    pub fn new(struct_name: String, function_path: String, bytecode: Vec<u8>) -> Self {
        Self {
            struct_name,
            patch_data: PatchData::new(function_path, bytecode),
        }
    }

    pub fn serialize<W: Write>(&self, w: &mut W) -> io::Result<()> {
        write_ue3_string(w, &self.struct_name)?;
        self.patch_data.serialize(w)
    }

    pub fn deserialize<R: Read>(r: &mut R) -> io::Result<Self> {
        Ok(Self {
            struct_name: read_ue3_string(r)?,
            patch_data: PatchData::deserialize(r)?,
        })
    }

    pub fn function_path(&self) -> &str {
        &self.patch_data.data_name
    }

    pub fn function_name(&self) -> &str {
        self.patch_data
            .data_name
            .rsplit('.')
            .next()
            .unwrap_or(&self.patch_data.data_name)
    }
}

#[derive(Debug, Clone)]
pub struct EnumPatchData {
    pub enum_name: String,
    pub enum_path_name: String,
    pub enum_values: Vec<String>,
}

impl EnumPatchData {
    pub fn new(enum_name: String, enum_path_name: String, enum_values: Vec<String>) -> Self {
        Self {
            enum_name,
            enum_path_name,
            enum_values,
        }
    }

    pub fn serialize<W: Write>(&self, w: &mut W) -> io::Result<()> {
        write_ue3_string(w, &self.enum_name)?;
        write_ue3_string(w, &self.enum_path_name)?;
        w.write_i32::<LittleEndian>(self.enum_values.len() as i32)?;
        for v in &self.enum_values {
            write_ue3_string(w, v)?;
        }
        Ok(())
    }

    pub fn deserialize<R: Read>(r: &mut R) -> io::Result<Self> {
        let enum_name = read_ue3_string(r)?;
        let enum_path_name = read_ue3_string(r)?;
        let n = r.read_i32::<LittleEndian>()? as usize;
        let mut vals = Vec::with_capacity(n);
        for _ in 0..n {
            vals.push(read_ue3_string(r)?);
        }
        Ok(Self {
            enum_name,
            enum_path_name,
            enum_values: vals,
        })
    }
}

#[derive(Debug, Clone, Default)]
pub struct PatchExport {
    pub class_index: i32,
    pub super_index: i32,
    pub outer_index: i32,
    pub object_name: String,
    pub archetype_index: i32,
    pub object_flags: u64,
    pub serial_size: i32,
    pub serial_offset: i32,
    pub export_flags: u32,
    pub generation_net_object_count: Vec<i32>,
    pub package_guid: [u32; 4],
    pub package_flags: u32,
}

impl PatchExport {
    pub fn serialize<W: Write>(&self, w: &mut W) -> io::Result<()> {
        w.write_i32::<LittleEndian>(self.class_index)?;
        w.write_i32::<LittleEndian>(self.super_index)?;
        w.write_i32::<LittleEndian>(self.outer_index)?;
        write_ue3_string(w, &self.object_name)?;
        w.write_i32::<LittleEndian>(self.archetype_index)?;
        w.write_u64::<LittleEndian>(self.object_flags)?;
        w.write_i32::<LittleEndian>(self.serial_size)?;
        w.write_i32::<LittleEndian>(self.serial_offset)?;
        w.write_u32::<LittleEndian>(self.export_flags)?;
        w.write_i32::<LittleEndian>(self.generation_net_object_count.len() as i32)?;
        for &v in &self.generation_net_object_count {
            w.write_i32::<LittleEndian>(v)?;
        }
        for &g in &self.package_guid {
            w.write_u32::<LittleEndian>(g)?;
        }
        w.write_u32::<LittleEndian>(self.package_flags)
    }

    pub fn deserialize<R: Read>(r: &mut R) -> io::Result<Self> {
        let class_index = r.read_i32::<LittleEndian>()?;
        let super_index = r.read_i32::<LittleEndian>()?;
        let outer_index = r.read_i32::<LittleEndian>()?;
        let object_name = read_ue3_string(r)?;
        let archetype_index = r.read_i32::<LittleEndian>()?;
        let object_flags = r.read_u64::<LittleEndian>()?;
        let serial_size = r.read_i32::<LittleEndian>()?;
        let serial_offset = r.read_i32::<LittleEndian>()?;
        let export_flags = r.read_u32::<LittleEndian>()?;
        let gen_count = r.read_i32::<LittleEndian>()? as usize;
        let mut gen_net = Vec::with_capacity(gen_count);
        for _ in 0..gen_count {
            gen_net.push(r.read_i32::<LittleEndian>()?);
        }
        let guid = [
            r.read_u32::<LittleEndian>()?,
            r.read_u32::<LittleEndian>()?,
            r.read_u32::<LittleEndian>()?,
            r.read_u32::<LittleEndian>()?,
        ];
        let package_flags = r.read_u32::<LittleEndian>()?;
        Ok(Self {
            class_index,
            super_index,
            outer_index,
            object_name,
            archetype_index,
            object_flags,
            serial_size,
            serial_offset,
            export_flags,
            generation_net_object_count: gen_net,
            package_guid: guid,
            package_flags,
        })
    }
}

#[derive(Debug, Clone, Default)]
pub struct PatchImport {
    pub class_package: String,
    pub class_name: String,
    pub outer_index: i32,
    pub object_name: String,
}

impl PatchImport {
    pub fn serialize<W: Write>(&self, w: &mut W) -> io::Result<()> {
        write_ue3_string(w, &self.class_package)?;
        write_ue3_string(w, &self.class_name)?;
        w.write_i32::<LittleEndian>(self.outer_index)?;
        write_ue3_string(w, &self.object_name)
    }

    pub fn deserialize<R: Read>(r: &mut R) -> io::Result<Self> {
        Ok(Self {
            class_package: read_ue3_string(r)?,
            class_name: read_ue3_string(r)?,
            outer_index: r.read_i32::<LittleEndian>()?,
            object_name: read_ue3_string(r)?,
        })
    }
}

#[derive(Debug, Clone)]
pub struct LinkerPatchData {
    pub package_name: String,
    pub names: Vec<String>,
    pub exports: Vec<PatchExport>,
    pub imports: Vec<PatchImport>,
    pub new_objects: Vec<PatchData>,
    pub modified_class_default_objects: Vec<PatchData>,
    pub modified_enums: Vec<EnumPatchData>,
    pub script_patches: Vec<ScriptPatchData>,
}

impl LinkerPatchData {
    pub fn new(pkg: String) -> Self {
        Self {
            package_name: pkg,
            names: Vec::new(),
            exports: Vec::new(),
            imports: Vec::new(),
            new_objects: Vec::new(),
            modified_class_default_objects: Vec::new(),
            modified_enums: Vec::new(),
            script_patches: Vec::new(),
        }
    }

    pub fn add_name(&mut self, n: String) {
        self.names.push(n);
    }

    pub fn add_export(&mut self, e: PatchExport) {
        self.exports.push(e);
    }
    pub fn add_import(&mut self, i: PatchImport) {
        self.imports.push(i);
    }

    pub fn add_new_object(&mut self, path: String, data: Vec<u8>) {
        self.new_objects.push(PatchData::new(path, data));
    }
    pub fn add_cdo_patch(&mut self, p: PatchData) {
        self.modified_class_default_objects.push(p);
    }
    pub fn add_enum_patch(&mut self, p: EnumPatchData) {
        self.modified_enums.push(p);
    }
    pub fn add_script_patch(&mut self, p: ScriptPatchData) {
        self.script_patches.push(p);
    }

    pub fn serialize<W: Write>(&self, w: &mut W) -> io::Result<()> {
        write_ue3_string(w, &self.package_name)?;

        w.write_i32::<LittleEndian>(self.names.len() as i32)?;
        for n in &self.names {
            write_ue3_string(w, n)?;
        }

        w.write_i32::<LittleEndian>(self.exports.len() as i32)?;
        for e in &self.exports {
            e.serialize(w)?;
        }

        w.write_i32::<LittleEndian>(self.imports.len() as i32)?;
        for i in &self.imports {
            i.serialize(w)?;
        }

        w.write_i32::<LittleEndian>(self.new_objects.len() as i32)?;
        for o in &self.new_objects {
            o.serialize(w)?;
        }

        w.write_i32::<LittleEndian>(self.modified_class_default_objects.len() as i32)?;
        for c in &self.modified_class_default_objects {
            c.serialize(w)?;
        }

        w.write_i32::<LittleEndian>(self.modified_enums.len() as i32)?;
        for e in &self.modified_enums {
            e.serialize(w)?;
        }

        w.write_i32::<LittleEndian>(self.script_patches.len() as i32)?;
        for p in &self.script_patches {
            p.serialize(w)?;
        }

        Ok(())
    }

    pub fn deserialize<R: Read>(r: &mut R) -> io::Result<Self> {
        let package_name = read_ue3_string(r)?;

        let nc = r.read_i32::<LittleEndian>()? as usize;
        let mut names = Vec::with_capacity(nc);
        for _ in 0..nc {
            names.push(read_ue3_string(r)?);
        }

        let ec = r.read_i32::<LittleEndian>()? as usize;
        let mut exports = Vec::with_capacity(ec);
        for _ in 0..ec {
            exports.push(PatchExport::deserialize(r)?);
        }

        let ic = r.read_i32::<LittleEndian>()? as usize;
        let mut imports = Vec::with_capacity(ic);
        for _ in 0..ic {
            imports.push(PatchImport::deserialize(r)?);
        }

        let no_c = r.read_i32::<LittleEndian>()? as usize;
        let mut new_objects = Vec::with_capacity(no_c);
        for _ in 0..no_c {
            new_objects.push(PatchData::deserialize(r)?);
        }

        let cdo_c = r.read_i32::<LittleEndian>()? as usize;
        let mut cdos = Vec::with_capacity(cdo_c);
        for _ in 0..cdo_c {
            cdos.push(PatchData::deserialize(r)?);
        }

        let en_c = r.read_i32::<LittleEndian>()? as usize;
        let mut enums = Vec::with_capacity(en_c);
        for _ in 0..en_c {
            enums.push(EnumPatchData::deserialize(r)?);
        }

        let sp_c = r.read_i32::<LittleEndian>()? as usize;
        let mut script_patches = Vec::with_capacity(sp_c);
        for _ in 0..sp_c {
            script_patches.push(ScriptPatchData::deserialize(r)?);
        }

        Ok(Self {
            package_name,
            names,
            exports,
            imports,
            new_objects,
            modified_class_default_objects: cdos,
            modified_enums: enums,
            script_patches,
        })
    }
}
pub fn compress_patch(patch: &LinkerPatchData) -> io::Result<(Vec<u8>, usize)> {
    let mut unc: Vec<u8> = Vec::new();
    patch.serialize(&mut unc)?;
    let unc_total = unc.len();

    let blocks: Vec<Vec<u8>> = unc
        .chunks(CHUNK_SIZE)
        .map(|chunk| {
            let mut enc = ZlibEncoder::new(Vec::new(), Compression::default());
            enc.write_all(chunk).unwrap();
            enc.finish().unwrap()
        })
        .collect();

    let n = blocks.len();
    let cmp_total: usize = blocks.iter().map(|b| b.len()).sum();

    let mut out = Vec::with_capacity(8 + 8 + n * 8 + cmp_total);

    out.extend_from_slice(&PACKAGE_FILE_TAG.to_le_bytes());
    out.extend_from_slice(&(COMPRESS_ZLIB as u32).to_le_bytes());
    out.extend_from_slice(&(unc_total as i32).to_le_bytes());
    out.extend_from_slice(&(cmp_total as i32).to_le_bytes());

    for (i, block) in blocks.iter().enumerate() {
        let unc_sz = if i == n - 1 {
            unc_total - i * CHUNK_SIZE
        } else {
            CHUNK_SIZE
        };
        out.extend_from_slice(&(block.len() as i32).to_le_bytes());
        out.extend_from_slice(&(unc_sz as i32).to_le_bytes());
    }

    for block in &blocks {
        out.extend_from_slice(block);
    }

    Ok((out, unc_total))
}

pub fn uncompressed_size_from(compressed: &[u8]) -> Option<u32> {
    if compressed.len() < 12 {
        return None;
    }
    let tag = u32::from_le_bytes(compressed[0..4].try_into().unwrap());
    if tag != PACKAGE_FILE_TAG {
        return None;
    }
    let sz = i32::from_le_bytes(compressed[8..12].try_into().unwrap());
    if sz < 0 { None } else { Some(sz as u32) }
}

pub fn load_patch_bin(data: &[u8]) -> io::Result<LinkerPatchData> {
    if data.len() < 16 {
        return Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "patch file too small",
        ));
    }

    // Verify PACKAGE_FILE_TAG
    let tag = u32::from_le_bytes(data[0..4].try_into().unwrap());
    if tag != PACKAGE_FILE_TAG {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "bad magic: expected 0x{:08X}, got 0x{:08X}",
                PACKAGE_FILE_TAG, tag
            ),
        ));
    }

    let unc_total = i32::from_le_bytes(data[8..12].try_into().unwrap()) as usize;

    let n_blocks = (unc_total + CHUNK_SIZE - 1) / CHUNK_SIZE;
    let hdrs_end = 16 + n_blocks * 8;
    if data.len() < hdrs_end {
        return Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "truncated block headers",
        ));
    }

    let mut unc: Vec<u8> = Vec::with_capacity(unc_total);
    let mut pos = hdrs_end;

    for i in 0..n_blocks {
        let h = 16 + i * 8;
        let cmp_sz = i32::from_le_bytes(data[h..h + 4].try_into().unwrap()) as usize;

        if pos + cmp_sz > data.len() {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                format!(
                    "block {} extends past EOF (need {} bytes at offset {})",
                    i, cmp_sz, pos
                ),
            ));
        }

        let mut dec = ZlibDecoder::new(&data[pos..pos + cmp_sz]);
        let mut blk = Vec::new();
        dec.read_to_end(&mut blk)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        unc.extend_from_slice(&blk);
        pos += cmp_sz;
    }

    if unc.len() != unc_total {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "decompressed {} bytes but header said {}",
                unc.len(),
                unc_total
            ),
        ));
    }

    LinkerPatchData::deserialize(&mut unc.as_slice())
}

fn export_serial_positions(raw: &[u8], header: &UpkHeader) -> Vec<(usize, usize)> {
    let mut pos = header.export_offset as usize;
    let mut result = Vec::with_capacity(header.export_count as usize);

    for _ in 0..header.export_count {
        if pos + 40 > raw.len() {
            break;
        }
        result.push((pos + 32, pos + 36));

        pos += 40;

        if header.p_ver < 543 {
            if pos + 4 > raw.len() {
                break;
            }
            let cnt = i32::from_le_bytes(raw[pos..pos + 4].try_into().unwrap_or([0; 4])) as usize;
            pos += 4 + cnt * 12;
        }

        if pos + 4 > raw.len() {
            break;
        }
        pos += 4;

        if pos + 4 > raw.len() {
            break;
        }
        let gc = i32::from_le_bytes(raw[pos..pos + 4].try_into().unwrap_or([0; 4])) as usize;
        pos += 4 + gc * 4;

        pos += 20;
    }

    result
}

pub fn apply_patches_to_upk(
    upk_raw: &[u8],
    header: &UpkHeader,
    pak: &UPKPak,
    patch: &LinkerPatchData,
) -> io::Result<Vec<u8>> {
    let serial_pos = export_serial_positions(upk_raw, header);
    let mut replacements: HashMap<usize, Vec<u8>> = HashMap::new();

    for sp in &patch.script_patches {
        let needle = sp.function_path().to_lowercase();
        let found = pak.export_table.iter().enumerate().find(|(i, _)| {
            pak.get_export_full_name((*i + 1) as i32)
                .to_lowercase()
                .contains(&needle)
        });

        let (exp_idx, exp) = match found {
            Some(f) => f,
            None => {
                eprintln!(
                    "  warn [apply]: no export for '{}' — skipped",
                    sp.function_path()
                );
                continue;
            }
        };

        let s = exp.serial_offset as usize;
        let e = s + exp.serial_size as usize;
        if e > upk_raw.len() {
            eprintln!(
                "  warn [apply]: export '{}' out of bounds — skipped",
                sp.function_path()
            );
            continue;
        }
        let blob = &upk_raw[s..e];

        let old_script = match extract_script_from_export_blob(blob, pak) {
            Some(sc) => sc,
            None => {
                eprintln!(
                    "  warn [apply]: cannot locate Script in '{}' — skipped",
                    sp.function_path()
                );
                continue;
            }
        };

        let count_bytes = (old_script.len() as i32).to_le_bytes();
        let search: Vec<u8> = count_bytes
            .iter()
            .chain(old_script.iter())
            .copied()
            .collect();
        let arr_off = match blob
            .windows(search.len())
            .position(|w| w == search.as_slice())
        {
            Some(p) => p,
            None => {
                eprintln!(
                    "  warn [apply]: cannot pin Script TArray in '{}' — skipped",
                    sp.function_path()
                );
                continue;
            }
        };

        let new_bc = &sp.patch_data.data;
        let mut new_blob = Vec::new();
        new_blob.extend_from_slice(&blob[..arr_off]);
        new_blob.extend_from_slice(&(new_bc.len() as i32).to_le_bytes());
        new_blob.extend_from_slice(new_bc);
        new_blob.extend_from_slice(&blob[arr_off + 4 + old_script.len()..]);

        println!(
            "  script patch '{}': {} → {} bytes",
            sp.function_path(),
            old_script.len(),
            new_bc.len()
        );
        replacements.insert(exp_idx, new_blob);
    }

    for cdo in &patch.modified_class_default_objects {
        let needle = cdo.data_name.to_lowercase();

        let found = pak.export_table.iter().enumerate().find(|(i, _)| {
            let full = pak.get_export_full_name((*i + 1) as i32);
            let path = pak.get_export_path_name((*i + 1) as i32);
            let inner = strip_pkg_prefix(&path);
            inner.to_lowercase() == needle || full.to_lowercase().contains(&needle)
        });

        let (exp_idx, exp) = match found {
            Some(f) => f,
            None => {
                eprintln!(
                    "  warn [apply]: no export matching CDO '{}' — skipped",
                    cdo.data_name
                );
                continue;
            }
        };

        println!(
            "  CDO patch '{}' → export '{}'  ({} → {} bytes)",
            cdo.data_name,
            pak.get_export_path_name((exp_idx + 1) as i32),
            exp.serial_size,
            cdo.data.len()
        );
        replacements.insert(exp_idx, cdo.data.clone());
    }

    if replacements.is_empty() {
        println!("  no exports matched — UPK unchanged");
        return Ok(upk_raw.to_vec());
    }

    let mut order: Vec<usize> = (0..pak.export_table.len())
        .filter(|&i| pak.export_table[i].serial_size > 0)
        .collect();
    order.sort_by_key(|&i| pak.export_table[i].serial_offset);

    let min_data_off = order
        .first()
        .map(|&i| pak.export_table[i].serial_offset as usize)
        .unwrap_or(upk_raw.len());
    let orig_data_end = order
        .last()
        .map(|&i| (pak.export_table[i].serial_offset + pak.export_table[i].serial_size) as usize)
        .unwrap_or(upk_raw.len());

    let mut new_file = upk_raw[..min_data_off].to_vec();
    let mut new_serial: Vec<(i32, i32)> = pak
        .export_table
        .iter()
        .map(|e| (e.serial_offset, e.serial_size))
        .collect();

    let mut cur_off = min_data_off;
    for &ei in &order {
        let exp = &pak.export_table[ei];
        let (blob, sz): (&[u8], usize) = if let Some(nb) = replacements.get(&ei) {
            (nb.as_slice(), nb.len())
        } else {
            let s = exp.serial_offset as usize;
            let sz = exp.serial_size as usize;
            (&upk_raw[s..s + sz], sz)
        };
        new_serial[ei] = (cur_off as i32, sz as i32);
        new_file.extend_from_slice(blob);
        cur_off += sz;
    }

    if orig_data_end < upk_raw.len() {
        new_file.extend_from_slice(&upk_raw[orig_data_end..]);
    }

    for (ei, (sz_pos, off_pos)) in serial_pos.iter().enumerate() {
        let (new_off, new_sz) = new_serial[ei];
        if *sz_pos + 4 <= new_file.len() {
            new_file[*sz_pos..*sz_pos + 4].copy_from_slice(&new_sz.to_le_bytes());
        }
        if *off_pos + 4 <= new_file.len() {
            new_file[*off_pos..*off_pos + 4].copy_from_slice(&new_off.to_le_bytes());
        }
    }

    Ok(new_file)
}

fn strip_pkg_prefix(path: &str) -> &str {
    if let Some(dot) = path.find('.') {
        &path[dot + 1..]
    } else {
        path
    }
}
