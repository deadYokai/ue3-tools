use std::{
    collections::HashMap,
    io::{self, Cursor, Read, Seek, SeekFrom, Write},
};
use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};
use flate2::{Compression, read::ZlibDecoder, write::ZlibEncoder};
use crate::{
    scriptdisasm::extract_script_from_export_blob,
    upkreader::{UPKPak, UpkHeader},
};

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
        if b.last() == Some(&0) { b.pop(); }
        Ok(String::from_utf8_lossy(&b).into_owned())
    } else {
        // UTF-16: len is -(char_count_including_null)
        let count = (-len) as usize;
        let mut chars = Vec::with_capacity(count);
        for _ in 0..count { chars.push(r.read_u16::<LittleEndian>()?); }
        if chars.last() == Some(&0) { chars.pop(); }
        Ok(String::from_utf16_lossy(&chars))
    }
}

/// Write TArray<BYTE>: `i32 count` + raw bytes.
fn write_bytes_array<W: Write>(w: &mut W, data: &[u8]) -> io::Result<()> {
    w.write_i32::<LittleEndian>(data.len() as i32)?;
    w.write_all(data)
}

/// Read TArray<BYTE>.
fn read_bytes_array<R: Read>(r: &mut R) -> io::Result<Vec<u8>> {
    let n = r.read_i32::<LittleEndian>()?;
    if n <= 0 { return Ok(Vec::new()); }
    let mut b = vec![0u8; n as usize];
    r.read_exact(&mut b)?;
    Ok(b)
}

// ─── FPatchData ───────────────────────────────────────────────────────────────
// C++: FString DataName;  TArray<BYTE> Data;

#[derive(Debug, Clone)]
pub struct PatchData {
    pub data_name: String,
    pub data: Vec<u8>,
}

impl PatchData {
    pub fn new(data_name: String, data: Vec<u8>) -> Self { Self { data_name, data } }

    pub fn serialize<W: Write>(&self, w: &mut W) -> io::Result<()> {
        write_ue3_string(w, &self.data_name)?;
        write_bytes_array(w, &self.data)
    }

    pub fn deserialize<R: Read>(r: &mut R) -> io::Result<Self> {
        Ok(Self { data_name: read_ue3_string(r)?, data: read_bytes_array(r)? })
    }
}

// ─── FScriptPatchData ─────────────────────────────────────────────────────────
// C++: FName StructName; (inherits FPatchData: FString DataName; TArray<BYTE> Data)
// operator<<: Ar << StructName << (FPatchData&)Patch
//   → StructName via FPatchBinaryWriter override → FString
//   → DataName as plain FString, Data as TArray<BYTE>

#[derive(Debug, Clone)]
pub struct ScriptPatchData {
    pub struct_name: String,
    pub patch_data: PatchData,
}

impl ScriptPatchData {
    pub fn new(struct_name: String, function_path: String, bytecode: Vec<u8>) -> Self {
        Self { struct_name, patch_data: PatchData::new(function_path, bytecode) }
    }

    pub fn serialize<W: Write>(&self, w: &mut W) -> io::Result<()> {
        write_ue3_string(w, &self.struct_name)?; // StructName first (FName→FString)
        self.patch_data.serialize(w)              // then DataName (FString) + Data (TArray<BYTE>)
    }

    pub fn deserialize<R: Read>(r: &mut R) -> io::Result<Self> {
        Ok(Self { struct_name: read_ue3_string(r)?, patch_data: PatchData::deserialize(r)? })
    }

    pub fn function_path(&self) -> &str { &self.patch_data.data_name }

    pub fn function_name(&self) -> &str {
        self.patch_data.data_name.rsplit('.').next().unwrap_or(&self.patch_data.data_name)
    }
}

// ─── FEnumPatchData ───────────────────────────────────────────────────────────
// C++: FName EnumName; FString EnumPathName; TArray<FName> EnumValues;
// All FName via override → FString.

#[derive(Debug, Clone)]
pub struct EnumPatchData {
    pub enum_name: String,
    pub enum_path_name: String,
    pub enum_values: Vec<String>,
}

impl EnumPatchData {
    pub fn new(enum_name: String, enum_path_name: String, enum_values: Vec<String>) -> Self {
        Self { enum_name, enum_path_name, enum_values }
    }

    pub fn serialize<W: Write>(&self, w: &mut W) -> io::Result<()> {
        write_ue3_string(w, &self.enum_name)?;
        write_ue3_string(w, &self.enum_path_name)?;
        w.write_i32::<LittleEndian>(self.enum_values.len() as i32)?;
        for v in &self.enum_values { write_ue3_string(w, v)?; }
        Ok(())
    }

    pub fn deserialize<R: Read>(r: &mut R) -> io::Result<Self> {
        let enum_name = read_ue3_string(r)?;
        let enum_path_name = read_ue3_string(r)?;
        let n = r.read_i32::<LittleEndian>()? as usize;
        let mut vals = Vec::with_capacity(n);
        for _ in 0..n { vals.push(read_ue3_string(r)?); }
        Ok(Self { enum_name, enum_path_name, enum_values: vals })
    }
}

// ─── FLinkerPatchData ─────────────────────────────────────────────────────────
// C++ serialize order (from UnScriptPatcher.cpp):
//   PackageName, Names, Exports, Imports,
//   NewObjects, ModifiedClassDefaultObjects, ModifiedEnums, ScriptPatches

#[derive(Debug, Clone)]
pub struct LinkerPatchData {
    pub package_name: String,
    pub names: Vec<String>,
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
            new_objects: Vec::new(),
            modified_class_default_objects: Vec::new(),
            modified_enums: Vec::new(),
            script_patches: Vec::new(),
        }
    }

    pub fn add_script_patch(&mut self, p: ScriptPatchData) { self.script_patches.push(p); }
    pub fn add_cdo_patch(&mut self, p: PatchData) { self.modified_class_default_objects.push(p); }
    pub fn add_enum_patch(&mut self, p: EnumPatchData) { self.modified_enums.push(p); }
    pub fn add_new_object(&mut self, path: String, data: Vec<u8>) {
        self.new_objects.push(PatchData::new(path, data));
    }
    pub fn add_name(&mut self, n: String) { self.names.push(n); }

    /// Serialize to the uncompressed binary stream read by FPatchBinaryReader.
    pub fn serialize<W: Write>(&self, w: &mut W) -> io::Result<()> {
        // 1. PackageName (FName → FString)
        write_ue3_string(w, &self.package_name)?;

        // 2. Names (TArray<FName → FString>)
        w.write_i32::<LittleEndian>(self.names.len() as i32)?;
        for n in &self.names { write_ue3_string(w, n)?; }

        // 3. Exports (TArray<FObjectExport>) — empty; tool never adds exports
        w.write_i32::<LittleEndian>(0)?;

        // 4. Imports (TArray<FObjectImport>) — empty
        w.write_i32::<LittleEndian>(0)?;

        // 5. NewObjects (TArray<FPatchData>)
        w.write_i32::<LittleEndian>(self.new_objects.len() as i32)?;
        for o in &self.new_objects { o.serialize(w)?; }

        // 6. ModifiedClassDefaultObjects (TArray<FPatchData>)
        w.write_i32::<LittleEndian>(self.modified_class_default_objects.len() as i32)?;
        for c in &self.modified_class_default_objects { c.serialize(w)?; }

        // 7. ModifiedEnums (TArray<FEnumPatchData>)
        w.write_i32::<LittleEndian>(self.modified_enums.len() as i32)?;
        for e in &self.modified_enums { e.serialize(w)?; }

        // 8. ScriptPatches (TArray<FScriptPatchData>)
        w.write_i32::<LittleEndian>(self.script_patches.len() as i32)?;
        for p in &self.script_patches { p.serialize(w)?; }

        Ok(())
    }

    pub fn deserialize<R: Read>(r: &mut R) -> io::Result<Self> {
        let package_name = read_ue3_string(r)?;

        let nc = r.read_i32::<LittleEndian>()? as usize;
        let mut names = Vec::with_capacity(nc);
        for _ in 0..nc { names.push(read_ue3_string(r)?); }

        let ec = r.read_i32::<LittleEndian>()?;
        if ec != 0 {
            return Err(io::Error::new(io::ErrorKind::Unsupported,
                format!("patch has {} Exports — full FObjectExport deserialize not implemented", ec)));
        }
        let ic = r.read_i32::<LittleEndian>()?;
        if ic != 0 {
            return Err(io::Error::new(io::ErrorKind::Unsupported,
                format!("patch has {} Imports — full FObjectImport deserialize not implemented", ic)));
        }

        let no_c = r.read_i32::<LittleEndian>()? as usize;
        let mut new_objects = Vec::with_capacity(no_c);
        for _ in 0..no_c { new_objects.push(PatchData::deserialize(r)?); }

        let cdo_c = r.read_i32::<LittleEndian>()? as usize;
        let mut cdos = Vec::with_capacity(cdo_c);
        for _ in 0..cdo_c { cdos.push(PatchData::deserialize(r)?); }

        let en_c = r.read_i32::<LittleEndian>()? as usize;
        let mut enums = Vec::with_capacity(en_c);
        for _ in 0..en_c { enums.push(EnumPatchData::deserialize(r)?); }

        let sp_c = r.read_i32::<LittleEndian>()? as usize;
        let mut script_patches = Vec::with_capacity(sp_c);
        for _ in 0..sp_c { script_patches.push(ScriptPatchData::deserialize(r)?); }

        Ok(Self {
            package_name, names, new_objects,
            modified_class_default_objects: cdos,
            modified_enums: enums,
            script_patches,
        })
    }
}

// ─── Compression ─────────────────────────────────────────────────────────────
// Matches UE3 FArchive::SerializeCompressed with GBaseCompressionMethod = COMPRESS_ZLIB.

const BLOCK_SIZE: usize = 0x20000; // 128 KiB — matches UE3 default chunk size

/// Serialize and compress a patch into the `.bin` format loaded by `FScriptPatcher::GetLinkerPatch`.
pub fn compress_patch(patch: &LinkerPatchData) -> io::Result<Vec<u8>> {
    let mut unc: Vec<u8> = Vec::new();
    patch.serialize(&mut unc)?;
    let unc_total = unc.len() as u32;

    let blocks: Vec<Vec<u8>> = unc.chunks(BLOCK_SIZE).map(|chunk| {
        let mut enc = ZlibEncoder::new(Vec::new(), Compression::default());
        enc.write_all(chunk).unwrap();
        enc.finish().unwrap()
    }).collect();

    let bcount = blocks.len();
    let comp_total: u32 = blocks.iter().map(|b| b.len() as u32).sum();

    let mut out = Vec::new();
    out.extend_from_slice(&unc_total.to_le_bytes());
    out.extend_from_slice(&comp_total.to_le_bytes());
    for (i, block) in blocks.iter().enumerate() {
        let unc_sz = if i == bcount - 1 { unc.len() - i * BLOCK_SIZE } else { BLOCK_SIZE };
        out.extend_from_slice(&(block.len() as u32).to_le_bytes());
        out.extend_from_slice(&(unc_sz as u32).to_le_bytes());
    }
    for block in &blocks { out.extend_from_slice(block); }
    Ok(out)
}

/// Decompress and deserialize a `.bin` patch file.
pub fn load_patch_bin(data: &[u8]) -> io::Result<LinkerPatchData> {
    if data.len() < 8 {
        return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "patch file too small"));
    }
    let unc_total = u32::from_le_bytes(data[0..4].try_into().unwrap()) as usize;
    let bcount = (unc_total + BLOCK_SIZE - 1) / BLOCK_SIZE;
    let hdrs_end = 8 + bcount * 8;
    if data.len() < hdrs_end {
        return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "truncated block headers"));
    }

    let mut unc: Vec<u8> = Vec::with_capacity(unc_total);
    let mut pos = hdrs_end;
    for i in 0..bcount {
        let h = 8 + i * 8;
        let csz = u32::from_le_bytes(data[h..h+4].try_into().unwrap()) as usize;
        let mut dec = ZlibDecoder::new(&data[pos..pos + csz]);
        let mut blk = Vec::new();
        dec.read_to_end(&mut blk)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        unc.extend_from_slice(&blk);
        pos += csz;
    }

    LinkerPatchData::deserialize(&mut unc.as_slice())
}

// ─── Offline UPK patching ─────────────────────────────────────────────────────

/// Returns `(serial_size_file_pos, serial_offset_file_pos)` for every export entry.
///
/// From `Export::read` the layout before `serial_size` is always:
///   class_index(4) + super_index(4) + outer_index(4)
///   + object_name(8) + archetype(4) + object_flags(8)
///   = 32 bytes fixed, for ALL versions.
/// The legacy_component_map (ver < 543) comes AFTER serial_offset, so
/// serial_size is always at entry_start + 32 and serial_offset at + 36.
fn export_serial_positions(raw: &[u8], header: &UpkHeader) -> Vec<(usize, usize)> {
    let mut pos = header.export_offset as usize;
    let mut result = Vec::with_capacity(header.export_count as usize);

    for _ in 0..header.export_count {
        if pos + 40 > raw.len() { break; }
        result.push((pos + 32, pos + 36));

        // Advance: fixed 40-byte prefix (32 + serial_size(4) + serial_offset(4))
        pos += 40;

        // Legacy component map only on ver < 543
        if header.p_ver < 543 {
            if pos + 4 > raw.len() { break; }
            let cnt = i32::from_le_bytes(raw[pos..pos+4].try_into().unwrap_or([0;4])) as usize;
            pos += 4 + cnt * 12; // each entry: FName(8) + i32(4)
        }

        if pos + 4 > raw.len() { break; }
        pos += 4; // export_flags

        if pos + 4 > raw.len() { break; }
        let gc = i32::from_le_bytes(raw[pos..pos+4].try_into().unwrap_or([0;4])) as usize;
        pos += 4 + gc * 4; // gen_count + gen_count * i32

        pos += 20; // package_guid(16) + package_flags(4)
    }

    result
}

/// Apply a `LinkerPatchData` to a raw UPK buffer, returning the (possibly resized) patched file.
///
/// For each script patch:
/// 1. Finds the export whose full name contains `function_path`.
/// 2. Extracts the current Script bytecode from the export blob.
/// 3. Locates the `TArray<BYTE>` (`i32 count` + bytes) in the blob.
/// 4. Replaces it with the new bytecode.
/// 5. Rebuilds the data section with updated `serial_size` / `serial_offset`.
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
            pak.get_export_full_name((*i + 1) as i32).to_lowercase().contains(&needle)
        });

        let (exp_idx, exp) = match found {
            Some(f) => f,
            None => {
                eprintln!("  warn [apply]: no export for '{}' — skipped", sp.function_path());
                continue;
            }
        };

        let s = exp.serial_offset as usize;
        let e = s + exp.serial_size as usize;
        if e > upk_raw.len() {
            eprintln!("  warn [apply]: export '{}' out of bounds — skipped", sp.function_path());
            continue;
        }
        let blob = &upk_raw[s..e];

        let old_script = match extract_script_from_export_blob(blob, pak) {
            Some(sc) => sc,
            None => {
                eprintln!("  warn [apply]: cannot locate Script in '{}' — skipped", sp.function_path());
                continue;
            }
        };

        // Find the exact TArray<BYTE> = [i32 count][bytes...] position in blob.
        let count_bytes = (old_script.len() as i32).to_le_bytes();
        let search: Vec<u8> = count_bytes.iter().chain(old_script.iter()).copied().collect();
        let tarray_off = match blob.windows(search.len()).position(|w| w == search.as_slice()) {
            Some(p) => p,
            None => {
                eprintln!("  warn [apply]: cannot pin Script TArray in '{}' — skipped", sp.function_path());
                continue;
            }
        };

        let new_bc = &sp.patch_data.data;
        let mut new_blob = Vec::new();
        new_blob.extend_from_slice(&blob[..tarray_off]);
        new_blob.extend_from_slice(&(new_bc.len() as i32).to_le_bytes());
        new_blob.extend_from_slice(new_bc);
        new_blob.extend_from_slice(&blob[tarray_off + 4 + old_script.len()..]);

        println!(
            "  patch '{}': {} → {} bytes",
            sp.function_path(), old_script.len(), new_bc.len()
        );
        replacements.insert(exp_idx, new_blob);
    }

    if replacements.is_empty() {
        println!("  no exports matched — UPK unchanged");
        return Ok(upk_raw.to_vec());
    }

    // ── Rebuild the file ──────────────────────────────────────────────────────

    // All exports with data, sorted by their original serial_offset.
    let mut order: Vec<usize> = (0..pak.export_table.len())
        .filter(|&i| pak.export_table[i].serial_size > 0)
        .collect();
    order.sort_by_key(|&i| pak.export_table[i].serial_offset);

    let min_data_off = order.first()
        .map(|&i| pak.export_table[i].serial_offset as usize)
        .unwrap_or(upk_raw.len());

    let orig_data_end = order.last()
        .map(|&i| (pak.export_table[i].serial_offset + pak.export_table[i].serial_size) as usize)
        .unwrap_or(upk_raw.len());

    // Start with a copy of the header + tables section (unchanged).
    let mut new_file = upk_raw[..min_data_off].to_vec();

    // new_serial: updated (serial_offset, serial_size) per export index.
    let mut new_serial: Vec<(i32, i32)> = pak.export_table.iter()
        .map(|e| (e.serial_offset, e.serial_size))
        .collect();

    let mut cur_off = min_data_off;
    for &ei in &order {
        let exp = &pak.export_table[ei];
        let (blob, sz) = if let Some(nb) = replacements.get(&ei) {
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

    // Trailing bytes after the last export (e.g. thumbnail data in some UPKs).
    if orig_data_end < upk_raw.len() {
        new_file.extend_from_slice(&upk_raw[orig_data_end..]);
    }

    // Patch serial_size (@ +32) and serial_offset (@ +36) in the export table.
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
