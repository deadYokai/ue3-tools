use std::{fmt, fs::File, io::{BufWriter, Cursor, Error, ErrorKind, Read, Result, Seek, Write}, path::{Path, PathBuf}};
use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};
use ron::ser::{to_string_pretty, PrettyConfig};
use serde::{Serialize, Deserialize};
use bitflags::bitflags;
use crate::{upkdecompress::CompressionMethod, upkprops::{self, Property, PropertyValue}};

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
pub struct Names
{
    pub n_len: i32,
    pub is_utf16: bool,
    pub name_bytes: Vec<u8>,
    pub name: String,
    n_fh: i32,
    n_fl: i32
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Export
{
    obj_type_ref: i32,
    parent_class_ref: i32,
    owner_ref: i32,
    name_tbl_idx: i32,
    name_count: i32, // if non-zero "_N" added to objName,
                     // where N = NameCount-1
    field6: i32,
    obj_flags_h: i32,
    obj_flags_l: i32,
    obj_filesize: i32,
    data_offset: i32,
    field11: i32,
    num_additional_fields: i32,
    field13: i32,
    field14: i32,
    field15: i32,
    field16: i32,
    field17: i32,
    unk_fields: Vec<i32>
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Import
{
    package_idx: i32,
    unk1: i32,
    obj_type_idx: i32,
    unk2: i32,
    owner_ref: i32,
    name_tbl_idx: i32,
    unk3: i32
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
    pub compression: CompressionMethod, 
    pub compressed_chunks: u32,
    pub package_source: i32,
    pub additional_packages: i32,
    pub texture_allocs: i32
}

#[derive(Debug, Serialize, Deserialize)]
pub struct UPKPak
{
    pub name_table: Vec<String>,
    pub export_table: Vec<Export>,
    pub import_table: Vec<Import>,
}

pub fn parse_upk(cursor: &mut Cursor<&Vec<u8>>, header: &UpkHeader) -> Result<UPKPak>
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
        let obj_type_ref = cursor.read_i32::<LittleEndian>()?;
        let parent_class_ref = cursor.read_i32::<LittleEndian>()?;
        let owner_ref = cursor.read_i32::<LittleEndian>()?;
        let name_tbl_idx = cursor.read_i32::<LittleEndian>()?;
        let name_count = cursor.read_i32::<LittleEndian>()?;
        let field6 = cursor.read_i32::<LittleEndian>()?;
        let obj_flags_h = cursor.read_i32::<LittleEndian>()?;
        let obj_flags_l = cursor.read_i32::<LittleEndian>()?;
        let obj_filesize = cursor.read_i32::<LittleEndian>()?;
        let data_offset = cursor.read_i32::<LittleEndian>()?;
        let field11 = cursor.read_i32::<LittleEndian>()?;
        let num_additional_fields = cursor.read_i32::<LittleEndian>()?;

        let mut unk_fields = Vec::new();
        for _ in 0..num_additional_fields {
            unk_fields.push(cursor.read_i32::<LittleEndian>()?);
        }

        let field13 = cursor.read_i32::<LittleEndian>()?;
        let field14 = cursor.read_i32::<LittleEndian>()?;
        let field15 = cursor.read_i32::<LittleEndian>()?;
        let field16 = cursor.read_i32::<LittleEndian>()?;
        let field17 = cursor.read_i32::<LittleEndian>()?;

        export_table.push(Export {
            obj_type_ref,
            parent_class_ref,
            owner_ref,
            name_tbl_idx,
            name_count,
            field6,
            obj_flags_h,
            obj_flags_l,
            obj_filesize,
            data_offset,
            field11,
            num_additional_fields,
            field13,
            field14,
            field15,
            field16,
            field17,
            unk_fields,
        });

    }

    // package_idx: i32,
    // unk1: i32,
    // obj_type_idx: i32,
    // unk2: i32,
    // owner_ref: i32,
    // name_tbl_idx: i32,
    // unk3: i32

    let mut import_table = Vec::new();

    cursor.set_position(import_offset as u64);
    for _ in 0..import_count
    {
        let package_idx = cursor.read_i32::<LittleEndian>()?;
        let unk1 = cursor.read_i32::<LittleEndian>()?;
        let obj_type_idx = cursor.read_i32::<LittleEndian>()?;
        let unk2 = cursor.read_i32::<LittleEndian>()?;
        let owner_ref = cursor.read_i32::<LittleEndian>()?;
        let name_tbl_idx = cursor.read_i32::<LittleEndian>()?;
        let unk3 = cursor.read_i32::<LittleEndian>()?;

        import_table.push(Import { package_idx, unk1, obj_type_idx, unk2, owner_ref, name_tbl_idx, unk3 });
    }

    Ok(UPKPak{name_table, export_table, import_table})
}

pub fn resolve_type_name(obj_type_ref: i32, pkg: &UPKPak) -> String {
    if obj_type_ref < 0 {
        let import_index = (-obj_type_ref - 1) as usize;
        if import_index < pkg.import_table.len() {
            let import = &pkg.import_table[import_index];
            if (import.name_tbl_idx as usize) < pkg.name_table.len() {
                return pkg.name_table[import.name_tbl_idx as usize].clone();
            }
        }
    } else if obj_type_ref > 0 {
        let export_index = (obj_type_ref - 1) as usize;
        if export_index < pkg.export_table.len() {
            let export = &pkg.export_table[export_index];
            if (export.name_tbl_idx as usize) < pkg.name_table.len() {
                return pkg.name_table[export.name_tbl_idx as usize].clone();
            }
        }
    }

    "unk".to_string()
}

fn export_full_path(pkg: &UPKPak, idx: usize) -> String {
    let mut path_parts = Vec::new();
    let mut current = Some(idx as i32 + 1);
    let mut first = true;

    while let Some(i) = current
    {
        if i <= 0
        {
            break;
        }

        let exp = &pkg.export_table[i as usize - 1];

        let mut name = pkg.name_table
            .get(exp.name_tbl_idx as usize)
            .cloned().unwrap_or_else(|| "<invalid>".to_string());


        if exp.name_count > 0
        {
            name = format!("{}_{}", name, exp.name_count - 1);
        }

        if first {
            let extension = resolve_type_name(exp.obj_type_ref, pkg);
            name = format!("{}.{}", name, extension);
            first = false;
        }
        path_parts.push(name);

        current = Some(exp.owner_ref);
    }

    path_parts.reverse();
    path_parts.join("/")

}

pub fn list_full_obj_paths(pkg: &UPKPak) -> Vec<String>
{
    pkg.export_table
        .iter()
        .enumerate()
        .map(|(idx, _)| export_full_path(pkg, idx))
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
        "SwfMovie" => {
            let buf_vec = buf.to_vec();
            let mut cursor = Cursor::new(&buf_vec);
            let mut props = get_obj_props(&mut cursor, pkg, false)?;

            let rawdata_find: &Property = props.iter().find(|s| s.name == "RawData").unwrap();
            let rawdata = rawdata_find.value.as_vec();
            // let rawdata = &rawdata_find.value;

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

                // rawdata.write_all(&mut writer)?;
                writer.flush()?;
            }

            if file_buffer.is_empty() {
                let mut out_file = File::create(path)?;
                new_path = path.to_path_buf();
                out_file.write_all(buf)?;
            } else {
                // let filtered: Vec<_> = props.iter().filter(|s| s.name != "RawData")
                //     .collect();
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
        let full_path = export_full_path(pkg, idx);

        if full_path.contains(path) || all {
            let file_path = out_dir.join(&full_path);
            if let Some(parent) = file_path.parent() {
                std::fs::create_dir_all(parent)?;
            }

            cursor.seek(std::io::SeekFrom::Start(exp.data_offset as u64))?;
            let mut buffer = vec![0u8; exp.obj_filesize as usize];
            cursor.read_exact(&mut buffer)?;

            let out_path = write_extracted_file(&file_path, &buffer, pkg)?; 

            println!("Exported \x1b[93m{}\x1b[0m (\x1b[33m{}\x1b[0m bytes) to\n\t \x1b[32m{}\x1b[0m", full_path, buffer.len(), out_path.display());
            found = true;
        }
    }

    if !found
    {
        println!("File {} not exists in package.", path);
    }

    Ok(())
}

pub fn read_name(cursor: &mut Cursor<&Vec<u8>>) -> Result<Names>
{
    let len = cursor.read_i32::<LittleEndian>()?;
    
    if len == 0
    {
        return Ok(Names{n_len: 0, is_utf16: false, name: "".to_string(), name_bytes: Vec::new(), n_fh: 0, n_fl: 0})
    }

    if len > 0
    {
        let mut buf = vec![0u8; len as usize];
        cursor.read_exact(&mut buf)?;

        let n_fh = cursor.read_i32::<LittleEndian>()?;
        let n_fl = cursor.read_i32::<LittleEndian>()?;

        if buf.last() == Some(&0)
        {
            buf.pop();
        }

        let name = buf.iter().map(|&b| b as char).collect::<String>(); // ISO-8859-1

        Ok(Names
        {
            n_len: len, is_utf16: false, name, name_bytes: buf, n_fh, n_fl
        })

    } else {
        let wchar_count = -len;
        let mut buf = vec![0u8; (wchar_count * 2) as usize];
        cursor.read_exact(&mut buf)?;

        let n_fh = cursor.read_i32::<LittleEndian>()?;
        let n_fl = cursor.read_i32::<LittleEndian>()?;

        let utf16: Vec<u16> = buf
            .chunks_exact(2)
            .map(|chunk| u16::from_le_bytes([chunk[0], chunk[1]]))
            .collect();

        let utf16_trimmed = match utf16.last()
        {
            Some(&0) => &utf16[..utf16.len() - 1],
            _ => &utf16[..]
        };

        let name = String::from_utf16(utf16_trimmed)
            .map_err(|_| Error::new(ErrorKind::InvalidData, "Invalid UTF16"))?;
        Ok(Names
        {
            n_len: wchar_count, is_utf16: true, name, name_bytes: buf, n_fh, n_fl
        })
    }
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
        writeln!(f, "Package Flags: (0x{:08x})", self.pak_flags)?;
        PackageFlags::from_bits_truncate(self.pak_flags).print_flags();
        writeln!(f, "Name Count: {}", self.name_count)?;
        writeln!(f, "Export Count: {}", self.export_count)?;
        writeln!(f, "Import Count: {}", self.import_count)?;
        if self.p_ver > 623 {
            writeln!(f, "Import/Export Guids pos: {}", self.import_export_guids_offset)?;
            writeln!(f, "Import Guids Count: {}", self.import_guids_count)?;
            writeln!(f, "Export Guids Count: {}", self.export_guids_count)?;
        } 
        if self.p_ver > 584{ 
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
        writeln!(f, "Compression Flags: {:#?}", self.compression)?;
        if self.compression != CompressionMethod::None {
            writeln!(f, "Num of compressed chunks: {}", self.compressed_chunks)?;
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
        let compression = 
            CompressionMethod::try_from(reader.read_u32::<LittleEndian>()?).unwrap();
        let compressed_chunks = reader.read_u32::<LittleEndian>()?;

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
            compression,
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
        
        if self.p_ver > 623 {
            writer.write_i32::<LittleEndian>(self.import_export_guids_offset)?;
            writer.write_u32::<LittleEndian>(self.import_guids_count)?;
            writer.write_u32::<LittleEndian>(self.export_guids_count)?;
        } 
        if self.p_ver > 584{ 
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
        writer.write_u32::<LittleEndian>(self.compression as u32)?;
        writer.write_u32::<LittleEndian>(self.compressed_chunks)?;
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

