use std::{collections::HashMap, fmt, fs::File, io::{BufWriter, Cursor, Error, ErrorKind, Read, Result, Seek, Write}, path::{Path, PathBuf}};

use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};
use ron::ser::{to_string_pretty, PrettyConfig};
use serde::{Serialize, Deserialize};
use bitflags::bitflags;
use crate::{upkdecompress::{CompressedChunk, CompressionMethod}, upkprops::{self, Property, PropertyValue}};

pub const PACKAGE_TAG: u32 = 0x9E2A83C1;

bitflags! {
    pub struct PackageFlags: u32 {
        const AllowDownload = 0x1;
        const ClientOptional = 0x2;
        const ServerSideOnly = 0x4;
        const Cooked = 0x8;
        const Unsecure = 0x10;
        const SavedWithNewerVersion = 0x20;
        const Need = 0x8000;
        const ContainsMap = 0x20000;
        const Trash = 0x40000;
        const DisallowLazyLoading = 0x100000;
        const ContainsScript = 0x200000;
        const ContainsDebugInfo = 0x400000;
        const RequireImportsAlreadyLoaded = 0x800000;
        const StoreCompressed = 0x2000000;
        const StoreFullyCompressed = 0x4000000;
        const ContainsFaceFxData = 0x10000000;
        const NoExportAllowed = 0x20000000;
        const StrippedSource = 0x40000000;
        const FilterEditorOnly = 0x80000000;
    }
}

impl PackageFlags {
    pub fn print_flags(&self) {
        for (flag, name) in [
            (PackageFlags::AllowDownload, "AllowDownload"),
            (PackageFlags::ClientOptional, "ClientOptional"),
            (PackageFlags::ServerSideOnly, "ServerSideOnly"),
            (PackageFlags::Cooked, "Cooked"),
            (PackageFlags::Unsecure, "Unsecure"),
            (PackageFlags::SavedWithNewerVersion, "SavedWithNewerVersion"),
            (PackageFlags::Need, "Need"),
            (PackageFlags::ContainsMap, "ContainsMap"),
            (PackageFlags::Trash, "Trash"),
            (PackageFlags::DisallowLazyLoading, "DisallowLazyLoading"),
            (PackageFlags::ContainsScript, "ContainsScript"),
            (PackageFlags::ContainsDebugInfo, "ContainsDebugInfo"),
            (PackageFlags::RequireImportsAlreadyLoaded, "RequireImportsAlreadyLoaded"),
            (PackageFlags::StoreCompressed, "StoreCompressed"),
            (PackageFlags::StoreFullyCompressed, "StoreFullyCompressed"),
            (PackageFlags::NoExportAllowed, "NoExportAllowed"),
            (PackageFlags::StrippedSource, "StrippedSource"),
            (PackageFlags::FilterEditorOnly, "FilterEditorOnly"),
        ] {
            if self.contains(flag){
                println!(" - {}", name);
            }
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct NameEntry
{
   pub name: String,
   pub flags: u64,
}

#[derive(Debug, Serialize, Deserialize, Hash, PartialEq, Eq)]
pub struct FName {
    name_index: i32,
    name_instance: i32
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Export
{
    class_index: i32,
    super_index: i32,
    outer_index: i32,
    object_name: FName,
    archetype: i32,
    object_flags: u64,
    serial_size: i32,
    serial_offset: i32,
    legacy_component_map: HashMap<FName, i32>,
    export_flags: u32,
    generation_net_object_count: Vec<i32>,
    package_guid: [i32; 4],
    package_flags: u32
}

impl Export {
    pub fn read(cursor: &mut Cursor<&Vec<u8>>, ver: i16) -> Result<Self>{
        let class_index = cursor.read_i32::<LittleEndian>()?;
        let super_index = cursor.read_i32::<LittleEndian>()?;
        let outer_index = cursor.read_i32::<LittleEndian>()?;

        let object_name = FName { 
            name_index: cursor.read_i32::<LittleEndian>()?, 
            name_instance: cursor.read_i32::<LittleEndian>()?
        };


        let archetype = cursor.read_i32::<LittleEndian>()?;

        let object_flags = cursor.read_u64::<LittleEndian>()?;

        let serial_size = cursor.read_i32::<LittleEndian>()?;
        let serial_offset = cursor.read_i32::<LittleEndian>()?;
        
        let mut legacy_component_map: HashMap<FName, i32> = HashMap::new();
        if ver < 543 {
            let count = cursor.read_i32::<LittleEndian>()?;
            for _ in 0..count {
                let k = FName { 
                    name_index: cursor.read_i32::<LittleEndian>()?, 
                    name_instance: cursor.read_i32::<LittleEndian>()?
                };
                let v = cursor.read_i32::<LittleEndian>()?;
                legacy_component_map.insert(k, v);
            }
        }

        let export_flags = cursor.read_u32::<LittleEndian>()?;
    
        let gen_count = cursor.read_i32::<LittleEndian>()?;
        let mut generation_net_object_count = Vec::with_capacity(gen_count as usize);
        for _ in 0..gen_count {
            generation_net_object_count.push(cursor.read_i32::<LittleEndian>()?);
        }

        let package_guid = [
            cursor.read_i32::<LittleEndian>()?,
            cursor.read_i32::<LittleEndian>()?,
            cursor.read_i32::<LittleEndian>()?,
            cursor.read_i32::<LittleEndian>()?
        ];
        let package_flags = cursor.read_u32::<LittleEndian>()?;

        Ok(Self {
            class_index,
            super_index,
            outer_index,
            object_name,
            archetype,
            object_flags,
            serial_size,
            serial_offset,
            legacy_component_map,
            export_flags,
            generation_net_object_count,
            package_guid,
            package_flags
        })
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Import
{
    pub class_package: FName,
    pub class_name: FName,
    pub outer_index: i32,
    pub object_name: FName
}

impl Import {
    pub fn read(cursor: &mut Cursor<&Vec<u8>>) -> Result<Self> {
        Ok(Self {
            class_package: FName {
                name_index: cursor.read_i32::<LittleEndian>()?,
                name_instance: cursor.read_i32::<LittleEndian>()?,
            },
            class_name: FName {
                name_index: cursor.read_i32::<LittleEndian>()?,
                name_instance: cursor.read_i32::<LittleEndian>()?,
            },
            outer_index: cursor.read_i32::<LittleEndian>()?,
            object_name: FName {
                name_index: cursor.read_i32::<LittleEndian>()?,
                name_instance: cursor.read_i32::<LittleEndian>()?,
            },
        })
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct GenerationInfo
{
    export_count: i32,
    name_count: i32,
    net_obj_count: i32
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct UpkHeader
{
    pub sign: u32,
    pub p_ver: i16,
    pub l_ver: i16,
    pub header_size: i32,
    pub path_len: i32,
    pub path: Vec<u8>,
    pub pak_flags: u32,
    pub name_count: i32,
    pub name_offset: i32,
    pub export_count: i32,
    pub export_offset: i32,
    pub import_count: i32,
    pub import_offset: i32,
    pub depends_offset: i32,
    pub import_export_guids_offset: i32,
    pub import_guids_count: u32,
    pub export_guids_count: u32,
    pub thumbnail_table_offest: u32,
    pub guid: [i32; 4],
    pub gen_count: i32,
    pub gens: Vec<GenerationInfo>,
    pub engine_ver: i32,
    pub cooker_ver: i32,
    pub compression_method: CompressionMethod, 
    pub compressed_chunks_count: u32,
    pub compressed_chunks: Vec<CompressedChunk>,
    pub package_source: i32,
    pub additional_packages: i32,
    pub texture_allocs: i32,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct UPKPak
{
    pub name_table: Vec<String>,
    pub export_table: Vec<Export>,
    pub import_table: Vec<Import>,
}

impl UPKPak { 
    pub fn parse_upk(cursor: &mut Cursor<&Vec<u8>>, header: &UpkHeader) -> Result<Self>
    {
        let name_count = header.name_count;
        let name_offset = header.name_offset;
        let export_count = header.export_count;
        let export_offset = header.export_offset;
        let import_count = header.import_count;
        let import_offset = header.import_offset;

        let mut name_table = Vec::new();
        cursor.set_position(name_offset as u64);
        for _ in 0..name_count
        {
            let name = read_name(cursor)?;
            name_table.push(name.name);
        }

        let mut export_table = Vec::new();
        cursor.set_position(export_offset as u64);
        for _ in 0..export_count
        {
            export_table.push(Export::read(cursor, header.p_ver)?);
        }

        let mut import_table = Vec::new();

        cursor.set_position(import_offset as u64);
        for _ in 0..import_count
        {
            import_table.push(Import::read(cursor)?);
        }

        Ok(Self{name_table, export_table, import_table})
    }

    pub fn fname_to_string(&self, fname: &FName) -> String {
        if let Some(name) = self.name_table.get(fname.name_index as usize) {
            if fname.name_instance > 0 {
                format!("{}_{}", name, fname.name_instance - 1)
            } else {
                name.clone()
            }
        } else {
            "<invalid>".to_string()
        }
    }

    pub fn get_import_class_name(&self, import_index: i32) -> String {
        let idx = (-import_index - 1) as usize;
        if let Some(import) = self.import_table.get(idx) {
            self.fname_to_string(&import.class_name)
        } else {
            "Class".to_string()
        }
    }

    pub fn get_export_class_name(&self, export_index: i32) -> String {
        let idx = (export_index - 1) as usize;
        if let Some(export) = self.export_table.get(idx) {
            self.fname_to_string(&export.object_name)
        } else {
            "Class".to_string()
        }
    }

    pub fn get_class_name(&self, class_index: i32) -> String {
        if class_index > 0 {
            let idx = (class_index - 1) as usize;
            if let Some(export) = self.export_table.get(idx) {
                self.fname_to_string(&export.object_name)
            } else {
                "Class".to_string()
            }
        } else if class_index < 0 {
            let idx = (-class_index - 1) as usize;
            if let Some(import) = self.import_table.get(idx) {
                self.fname_to_string(&import.object_name)
            } else {
                "Class".to_string()
            }
        } else {
            "Class".to_string()
        }
    }

    pub fn get_import_path_name(&self, import_index: i32) -> String {
        let mut result = String::new();
        let mut linker_index = -import_index - 1;

        while linker_index != 0 {
            let (object_name, outer_index, is_subobject) = 
                if linker_index >= 0 {
                    let idx = linker_index as usize;
                    if let Some(import) = self.import_table.get(idx) {
                        let is_subobj = !result.is_empty() 
                            && self.fname_to_string(&import.class_name) != "Package"
                            && self.is_package_outer(import.outer_index);
                        
                        (self.fname_to_string(&import.object_name), 
                         import.outer_index, 
                         is_subobj)
                    } else {
                        break;
                    }
                } else {
                    let idx = (-linker_index - 1) as usize;
                    if let Some(export) = self.export_table.get(idx) {
                        let is_subobj = !result.is_empty()
                            && self.get_class_name(-linker_index) != "Package"
                            && self.is_package_outer(export.outer_index);
                        
                        (self.fname_to_string(&export.object_name),
                         export.outer_index,
                         is_subobj)
                    } else {
                        break;
                    }
                };

            if !result.is_empty() {
                result = if is_subobject {
                    format!(":{}", result)
                } else {
                    format!(".{}", result)
                };
            }

            result = format!("{}{}", object_name, result);
            linker_index = outer_index;
        }

        result
    }

    pub fn get_export_path_name(&self, export_index: i32) -> String {
        let mut result = String::new();
        let mut linker_index = export_index;

        while linker_index != 0 {
            let idx = (linker_index - 1) as usize;
            if let Some(export) = self.export_table.get(idx) {
                if !result.is_empty() {
                    let is_subobject = self.get_class_name(linker_index) != "Package"
                        && self.is_package_outer(export.outer_index);
                    
                    result = if is_subobject {
                        format!(":{}", result)
                    } else {
                        format!(".{}", result)
                    };
                }

                result = format!("{}{}", 
                    self.fname_to_string(&export.object_name), 
                    result);
                linker_index = export.outer_index;
            } else {
                break;
            }
        }

        result
    }

    pub fn get_import_full_name(&self, import_index: i32) -> String {
        let idx = (-import_index - 1) as usize;
        if let Some(import) = self.import_table.get(idx) {
            let class_name = self.fname_to_string(&import.class_name);
            let path_name = self.get_import_path_name(import_index);
            format!("{} {}", class_name, path_name)
        } else {
            "<invalid>".to_string()
        }
    }

    pub fn get_export_full_name(&self, export_index: i32) -> String {
        let idx = (export_index - 1) as usize;
        if let Some(export) = self.export_table.get(idx) {
            let class_name = self.get_class_name(export.class_index);
            let path_name = self.get_export_path_name(export_index);
            format!("{} {}", class_name, path_name)
        } else {
            "<invalid>".to_string()
        }
    }

    fn is_package_outer(&self, outer_index: i32) -> bool {
        if outer_index == 0 {
            return true;
        }
        
        if outer_index > 0 {
            self.get_export_class_name(outer_index) == "Package"
        } else {
            self.get_import_class_name(outer_index) == "Package"
        }
    }
    
    fn ue_name_to_path(full_name: &str) -> String {
        let parts: Vec<&str> = full_name.splitn(2, ' ').collect();

        if parts.len() != 2 {
            return full_name.replace(&[':', '.'][..], "/");
        }

        let class_name = parts[0];
        let path_name = parts[1];
        let mut path_parts: Vec<String> = path_name
            .split(&['.', ':'][..])
            .map(|s| s.to_string())
            .collect();

        if let Some(last) = path_parts.last_mut() {
            *last = format!("{}.{}", last, class_name);
        }

        path_parts.join("/")
    }

    fn ue_name_to_path_class_first(full_name: &str) -> String {
        let parts: Vec<&str> = full_name.splitn(2, ' ').collect();

        if parts.len() != 2 {
            return full_name.replace(&[':', '.'][..], "/");
        }

        let class_name = parts[0];
        let path_name = parts[1];

        let path_parts: Vec<&str> = path_name.split(&['.', ':'][..]).collect();

        std::iter::once(class_name)
            .chain(path_parts.iter().copied())
            .collect::<Vec<_>>()
            .join("/")
    }
}

pub fn list_full_obj_paths(pkg: &UPKPak) -> Vec<String> {
    (0..pkg.export_table.len())
        .map(|idx| pkg.get_export_full_name((idx + 1) as i32))
        .collect()
}

pub fn write_extracted_file(path: &Path, buf: &[u8], pkg: &UPKPak) -> Result<PathBuf> {    
    let ext = path.extension().and_then(|s| s.to_str()).unwrap();
    let name = path.file_stem().and_then(|s| s.to_str()).unwrap();
    let dir = path.parent().unwrap();
    let mut new_path = dir.join(name);

    match ext {
        "ObjectReferencer" => {
            let buf_vec = buf.to_vec();
            let mut cursor = Cursor::new(&buf_vec);
            let props = get_obj_props(&mut cursor, pkg, false)?;
            let config = PrettyConfig::new().struct_names(true);
            let data = (format!("{}.{}", name, ext), &props);
            let ron_string = to_string_pretty(&data, config).unwrap();

            new_path = new_path.with_extension("ron");
            let mut ron_file = File::create(&new_path)?;
            writeln!(ron_file, "{ron_string}")?;
        },
        "SwfMovie" | "GFxMovieInfo" => {
            let buf_vec = buf.to_vec();
            let mut cursor = Cursor::new(&buf_vec);
            let mut props = get_obj_props(&mut cursor, pkg, false)?;

            let rawdata_find: &Property = props.iter().find(|s| s.name == "RawData").unwrap();
            let rawdata = rawdata_find.value.as_vec();

            let mut file_buffer = Vec::<u8>::new();
            
            {
                let mut writer = BufWriter::new(&mut file_buffer);

                if let Some(data) = rawdata {
                    for b in data.iter() {
                        if let Some(byte) = b.as_byte() {
                            writer.write_u8(byte)?;
                        }
                    }
                    
                }

                writer.flush()?;
            }

            if file_buffer.is_empty() {
                let mut out_file = File::create(path)?;
                new_path = path.to_path_buf();
                out_file.write_all(buf)?;
            } else {
                for prop in props.iter_mut() {
                    if prop.name == "RawData" {
                        prop.value = PropertyValue::String(format!("{}.gfx", name));
                    }
                }
                let config = PrettyConfig::new().struct_names(true);
                let data = (format!("{}.{}", name, ext), &props);
                let ron_string = to_string_pretty(&data, config).unwrap();

                let mut ron_file = File::create(new_path.with_extension("ron"))?;
                writeln!(ron_file, "{ron_string}")?;

                new_path = new_path.with_extension("gfx");
                let mut file = File::create(&new_path)?;
                file.write_all(&file_buffer)?;
            }
        }
        _ => {
            let mut out_file = File::create(path)?;
            new_path = path.to_path_buf();
            out_file.write_all(buf)?;
        }
    }

    Ok(new_path)
}

pub fn extract_by_name(cursor: &mut Cursor<Vec<u8>>, pkg: &UPKPak, path: &str, out_dir: &Path, all: bool) -> Result<()> {

    let mut found = false;

    for (idx, exp) in pkg.export_table.iter().enumerate() {
        let full_name = pkg.get_export_full_name((idx + 1) as i32);
        
        let fs_path = UPKPak::ue_name_to_path(&full_name);
        
        if fs_path.contains(path) || full_name.contains(path) || all {
            let exp = &pkg.export_table[idx];
            
            let file_path = out_dir.join(&fs_path);
            if let Some(parent) = file_path.parent() {
                std::fs::create_dir_all(parent)?;
            }

            cursor.seek(std::io::SeekFrom::Start(exp.serial_offset as u64))?;
            let mut buffer = vec![0u8; exp.serial_size as usize];
            cursor.read_exact(&mut buffer)?;

            let out_path = write_extracted_file(&file_path, &buffer, pkg)?;

            println!(
                "Exported \x1b[93m{}\x1b[0m (\x1b[33m{}\x1b[0m bytes) to\n\t \x1b[32m{}\x1b[0m",
                full_name,
                buffer.len(),
                out_path.display()
            );
            found = true;
        }
    }

    if !found
    {
        println!("File {} not exists in package.", path);
    }

    Ok(())
}

pub fn read_name(cursor: &mut Cursor<&Vec<u8>>) -> Result<NameEntry>
{
    let length = cursor.read_i32::<LittleEndian>()?;

    let name = if length < 0 {

        let abs_length = (-length) as usize;
        let mut u16_chars = vec![0u16; abs_length];
        for i in 0..abs_length {
            u16_chars[i] = cursor.read_u16::<LittleEndian>()?;
        }
        String::from_utf16(&u16_chars[..abs_length.saturating_sub(1)])
            .unwrap_or_else(|_| String::from("<invalid_utf16>"))
    } else {
        let length = length as usize;
        let mut bytes = vec![0u8; length];
        cursor.read_exact(&mut bytes)?;
        let name: String = bytes[..length.saturating_sub(1)]
            .iter()
            .map(|&b| b as char)
            .collect();

        name
    };

    let flags = cursor.read_u64::<LittleEndian>()?;

    Ok(NameEntry { name, flags })
}

pub fn read_string(cursor: &mut Cursor<&Vec<u8>>) -> Result<String>
{
    let len = cursor.read_i32::<LittleEndian>()?;
    if len == 0
    {
        return Ok("".to_string());
    }

    if len > 0
    {
        let mut buf = vec![0u8; len as usize];
        cursor.read_exact(&mut buf)?;

        if buf.last() == Some(&0)
        {
            buf.pop();
        }

        Ok(buf.iter().map(|&b| b as char).collect::<String>()) // not utf8 but ISO-8859-1
    } else {
        let wchar_count = -len;
        let mut buf = vec![0u8; (wchar_count * 2) as usize];
        cursor.read_exact(&mut buf)?;

        let utf16: Vec<u16> = buf
            .chunks_exact(2)
            .map(|chunk| u16::from_le_bytes([chunk[0], chunk[1]]))
            .collect();

        let utf16_trimmed = match utf16.last()
        {
            Some(&0) => &utf16[..utf16.len() - 1],
            _ => &utf16[..]
        };

        String::from_utf16(utf16_trimmed)
            .map_err(|_| Error::new(ErrorKind::InvalidData, "Invalid UTF16"))
    }
}

pub fn get_obj_props(
    cursor: &mut Cursor<&Vec<u8>>,
    upk: &UPKPak,
    print_out: bool
) -> Result<Vec<Property>>
{
    let mut props = Vec::new();
    while let Some(prop) = upkprops::parse_property(cursor, upk).expect("get_obj_props") {
        let start_pos = cursor.position();
        
        if print_out {
            println!("{:?}", prop);
        }

        props.push(prop);

        if cursor.position() >= cursor.seek(std::io::SeekFrom::End(0))?{
            break;
        }
        cursor.seek(std::io::SeekFrom::Start(start_pos))?;
    }

    Ok(props)    
}

impl fmt::Display for UpkHeader 
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result 
    {
        writeln!(f, "Package Signature: {:x?}", self.sign)?;
        writeln!(f, "Package Version: {}", self.p_ver)?;
        writeln!(f, "Licensee Version: {}", self.l_ver)?;
        writeln!(f, "Header Size: {}", self.header_size)?;
        writeln!(f, "Folder: {:?}", String::from_utf8_lossy(&self.path))?;
        writeln!(f, "Package Flags (0x{:08x}):", self.pak_flags)?;
        PackageFlags::from_bits_truncate(self.pak_flags).print_flags();
        writeln!(f, "Name Count: {}", self.name_count)?;
        writeln!(f, "Export Count: {}", self.export_count)?;
        writeln!(f, "Import Count: {}", self.import_count)?;
        if self.p_ver >= 623 {
            writeln!(f, "Import/Export Guids pos: {}", self.import_export_guids_offset)?;
            writeln!(f, "Import Guids Count: {}", self.import_guids_count)?;
            writeln!(f, "Export Guids Count: {}", self.export_guids_count)?;
        } 
        if self.p_ver >= 584{ 
            writeln!(f, "Thumbnail table pos: {}", self.thumbnail_table_offest)?;
        }
        writeln!(f, "GUID: {:x?}", self.guid)?;
        if self.gen_count > 0
        {
            writeln!(f, "Generations (Count={}):", self.gen_count)?;
        }
        for (i, gens) in self.gens.iter().enumerate()
        {
            writeln!(
                f, 
                " - Gen {}:\n   * Exports = {}\n   * Names   = {}\n   * NetObjs = {}", 
                i, gens.export_count, gens.name_count, gens.net_obj_count
            )?;
        }
        writeln!(f, "Engine Version: {}", self.engine_ver)?;
        writeln!(f, "Cooker Version: {}", self.cooker_ver)?;
        writeln!(f, "Compression Flags: {:#?}", self.compression_method)?;
        if self.compression_method != CompressionMethod::None {
            writeln!(f, "Num of compressed chunks: {}", self.compressed_chunks_count)?;
            for (i, c) in self.compressed_chunks.iter().enumerate(){
                writeln!(
                    f, 
                    " - Chunk {}:\n\
                    \x20  * Decompressed offset = {}\n\
                    \x20  * Decompressed size   = {}\n\
                    \x20  * Compressed offset   = {}\n\
                    \x20  * Compressed size     = {}", 
                    i, c.decompressed_offset,
                    c.decompressed_size,
                    c.compressed_offset,
                    c.compressed_size
                )?;
            }
        }
        
        writeln!(f, "Package Source: {}", self.package_source)?;

        if self.p_ver >= 516 {
            writeln!(f, "Additional packages: {}", self.additional_packages)?;
        }

        if self.p_ver >= 767 {
            writeln!(f, "Texture Allocations: {}", self.texture_allocs)?;
        }

        Ok(())
    }
}

impl UpkHeader {
    pub fn read<R: Read + Seek>(mut reader: R) -> Result<Self>
    {
        let sign = reader.read_u32::<LittleEndian>()?;
        if sign != PACKAGE_TAG
        {
            return Err(Error::new(ErrorKind::InvalidData, format!("Invalid file signature, sig=0x{:X}", sign)));
        }

        let p_ver = reader.read_i16::<LittleEndian>()?;
        let l_ver = reader.read_i16::<LittleEndian>()?;
        let header_size = reader.read_i32::<LittleEndian>()?;

        let path_len = reader.read_i32::<LittleEndian>()?;
        let mut rfl = path_len;
        if path_len < 0
        {
            rfl = path_len * -2; // needed if utf16
        }
        let mut path = vec![0u8; rfl as usize];
        reader.read_exact(&mut path)?;

        let pak_flags = reader.read_u32::<LittleEndian>()?;

        let name_count = reader.read_i32::<LittleEndian>()?;
        let name_offset = reader.read_i32::<LittleEndian>()?;
        let export_count = reader.read_i32::<LittleEndian>()?;
        let export_offset = reader.read_i32::<LittleEndian>()?;
        let import_count = reader.read_i32::<LittleEndian>()?;
        let import_offset = reader.read_i32::<LittleEndian>()?;
        let depends_offset = reader.read_i32::<LittleEndian>()?;

        if import_count <= 0 || name_count <= 0 || export_count <= 0
        {
            return Err(Error::new(ErrorKind::InvalidData, "Corrupted pak"));
        }
        
        let mut import_export_guids_offset = -1;
        let mut import_guids_count = 0;
        let mut export_guids_count = 0;
        let mut thumbnail_table_offest = 0;
        
        if p_ver >= 623 {
            import_export_guids_offset = reader.read_i32::<LittleEndian>()?;
            import_guids_count = reader.read_u32::<LittleEndian>()?;
            export_guids_count = reader.read_u32::<LittleEndian>()?;
        }

        if p_ver >= 584{ 
            thumbnail_table_offest = reader.read_u32::<LittleEndian>()?;
        }

        let guid =
            [
            reader.read_i32::<LittleEndian>()?,
            reader.read_i32::<LittleEndian>()?,
            reader.read_i32::<LittleEndian>()?,
            reader.read_i32::<LittleEndian>()?,
            ];

        let gen_count = reader.read_i32::<LittleEndian>()?;
        let mut gens = Vec::with_capacity(gen_count as usize);

        for _ in 0..gen_count
        {
            gens.push(
                GenerationInfo
                {
                    export_count: reader.read_i32::<LittleEndian>()?,
                    name_count: reader.read_i32::<LittleEndian>()?,
                    net_obj_count: reader.read_i32::<LittleEndian>()?
                }
            );
        }

        let engine_ver = reader.read_i32::<LittleEndian>()?;
        let cooker_ver = reader.read_i32::<LittleEndian>()?;
        let compression_method = 
            CompressionMethod::try_from(reader.read_u32::<LittleEndian>()?).unwrap();
        let compressed_chunks_count = reader.read_u32::<LittleEndian>()?;
        let mut compressed_chunks: Vec<CompressedChunk> = Vec::with_capacity(compressed_chunks_count as usize);

        for _ in 0..compressed_chunks_count {
            compressed_chunks.push(CompressedChunk{
                decompressed_offset: reader.read_u32::<LittleEndian>()?,
                decompressed_size: reader.read_u32::<LittleEndian>()?,
                compressed_offset: reader.read_u32::<LittleEndian>()?,
                compressed_size: reader.read_u32::<LittleEndian>()?,
            });
        }

        let package_source = reader.read_i32::<LittleEndian>()?;

        let mut additional_packages = -1;
        let mut texture_allocs = -1;

        if p_ver >= 516 {
            additional_packages = reader.read_i32::<LittleEndian>()?;
        }

        if p_ver >= 767 {
            texture_allocs = reader.read_i32::<LittleEndian>()?;
        }

        let header = UpkHeader
        {
            sign,
            p_ver,
            l_ver,
            header_size,
            path_len,
            path,
            pak_flags,
            name_count,
            name_offset,
            export_count,
            export_offset,
            import_count,
            import_offset,
            depends_offset,
            import_export_guids_offset,
            import_guids_count,
            export_guids_count,
            thumbnail_table_offest,
            guid,
            gen_count,
            gens,
            engine_ver,
            cooker_ver,
            compression_method,
            compressed_chunks_count,
            compressed_chunks,
            package_source,
            additional_packages,
            texture_allocs
        };

        Ok(header)
    }

    pub fn write<R: Write + Seek>(&self, mut writer: R) -> Result<()>
    {
        writer.write_u32::<LittleEndian>(self.sign)?;
        writer.write_i16::<LittleEndian>(self.p_ver)?;
        writer.write_i16::<LittleEndian>(self.l_ver)?;
        writer.write_i32::<LittleEndian>(self.header_size)?;
        writer.write_i32::<LittleEndian>(self.path_len)?;
        writer.write_all(&self.path)?;
        writer.write_u32::<LittleEndian>(self.pak_flags)?;
        writer.write_i32::<LittleEndian>(self.name_count)?;
        writer.write_i32::<LittleEndian>(self.name_offset)?;
        writer.write_i32::<LittleEndian>(self.export_count)?;
        writer.write_i32::<LittleEndian>(self.export_offset)?;
        writer.write_i32::<LittleEndian>(self.import_count)?;
        writer.write_i32::<LittleEndian>(self.import_offset)?;
        writer.write_i32::<LittleEndian>(self.depends_offset)?;
        
        if self.p_ver >= 623 {
            writer.write_i32::<LittleEndian>(self.import_export_guids_offset)?;
            writer.write_u32::<LittleEndian>(self.import_guids_count)?;
            writer.write_u32::<LittleEndian>(self.export_guids_count)?;
        } 
        if self.p_ver >= 584{ 
            writer.write_u32::<LittleEndian>(self.thumbnail_table_offest)?;
        }

        for v in &self.guid {
            writer.write_i32::<LittleEndian>(*v)?;
        }

        writer.write_i32::<LittleEndian>(self.gens.len() as i32)?;

        for g in &self.gens {
            writer.write_i32::<LittleEndian>(g.export_count)?;
            writer.write_i32::<LittleEndian>(g.name_count)?;
            writer.write_i32::<LittleEndian>(g.net_obj_count)?;
        }

        writer.write_i32::<LittleEndian>(self.engine_ver)?;
        writer.write_i32::<LittleEndian>(self.cooker_ver)?;
        writer.write_u32::<LittleEndian>(self.compression_method as u32)?;
        writer.write_u32::<LittleEndian>(self.compressed_chunks_count)?;
   
        if self.compressed_chunks_count > 0 {
            for c in &self.compressed_chunks {
                writer.write_u32::<LittleEndian>(c.decompressed_offset)?;
                writer.write_u32::<LittleEndian>(c.decompressed_size)?;
                writer.write_u32::<LittleEndian>(c.compressed_offset)?;
                writer.write_u32::<LittleEndian>(c.compressed_size)?;
            }
        }

        writer.write_i32::<LittleEndian>(self.package_source)?;

        if self.p_ver >= 516 {
            writer.write_i32::<LittleEndian>(self.additional_packages)?;
        }

        if self.p_ver >= 767 {
            writer.write_i32::<LittleEndian>(self.texture_allocs)?;
        }

        Ok(())
    }
}

