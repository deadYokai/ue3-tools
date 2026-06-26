use std::{
    collections::HashMap,
    fmt,
    fs::File,
    io::{Cursor, Error, ErrorKind, Read, Result, Seek, Write},
    path::{Path, PathBuf},
};

use crate::{
    native::{NativePayload, NativeRead, NativeReadCtx, NativeRegistry},
    pseudo::EmitInput,
    schemadb::{ResolvedRef, SchemaDb},
    upkprops::{self, Property, PropertyCtx, PropertyValue, parse_property_ctx},
    utils::decompress::{CompressedChunk, CompressionMethod},
    versions::{
        PACKAGE_FILE_TAG, PKG_FILTER_EDITOR_ONLY, VER_ADDED_CROSSLEVEL_REFERENCES,
        VER_ADDED_LINKER_DEPENDENCIES, VER_ADDED_PACKAGE_COMPRESSION_SUPPORT,
        VER_ADDITIONAL_COOK_PACKAGE_SUMMARY, VER_ASSET_THUMBNAILS_IN_PACKAGES,
        VER_FOBJECTEXPORT_EXPORTFLAGS, VER_LINKERFREE_PACKAGEMAP,
        VER_MOVED_EXPORTIMPORTMAPS_ADDED_TOTALHEADERSIZE, VER_NETINDEX_STORED_AS_INT,
        VER_PACKAGEFILESUMMARY_CHANGE, VER_PACKAGEFILESUMMARY_CHANGE_COOK_VER_ADDED,
        VER_REMOVED_COMPONENT_MAP, VER_TEXTURE_PREALLOCATION,
    },
};
use bitflags::bitflags;
use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};
use ron::ser::{PrettyConfig, to_string_pretty};
use serde::{Deserialize, Serialize};

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
            (
                PackageFlags::RequireImportsAlreadyLoaded,
                "RequireImportsAlreadyLoaded",
            ),
            (PackageFlags::StoreCompressed, "StoreCompressed"),
            (PackageFlags::StoreFullyCompressed, "StoreFullyCompressed"),
            (PackageFlags::NoExportAllowed, "NoExportAllowed"),
            (PackageFlags::StrippedSource, "StrippedSource"),
            (PackageFlags::FilterEditorOnly, "FilterEditorOnly"),
        ] {
            if self.contains(flag) {
                println!(" - {}", name);
            }
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct NameEntry {
    pub name: String,
    pub flags: u64,
}

#[derive(Debug, Serialize, Deserialize, Hash, PartialEq, Eq, Clone)]
pub struct FName {
    pub name_index: i32,
    pub name_instance: i32,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Export {
    pub class_index: i32,
    pub super_index: i32,
    pub outer_index: i32,
    pub object_name: FName,
    pub archetype: i32,
    pub object_flags: u64,
    pub serial_size: i32,
    pub serial_offset: i32,
    pub legacy_component_map: HashMap<FName, i32>,
    pub export_flags: u32,
    pub generation_net_object_count: Vec<i32>,
    pub package_guid: [i32; 4],
    pub package_flags: u32,
}

pub fn resolve_object_refs(props: &mut Vec<Property>, pkg: &UPKPak) {
    for prop in props.iter_mut() {
        resolve_value(&mut prop.value, pkg);
    }
}

fn resolve_value(val: &mut PropertyValue, pkg: &UPKPak) {
    match val {
        PropertyValue::Object(idx) => {
            let name = if *idx > 0 {
                pkg.get_export_full_name(*idx)
            } else if *idx < 0 {
                pkg.get_import_full_name(*idx)
            } else {
                "None".to_string()
            };
            *val = PropertyValue::ObjectRef(name);
        }
        PropertyValue::Array(elements) => {
            for el in elements.iter_mut() {
                resolve_value(el, pkg);
            }
        }
        PropertyValue::Struct(fields) => {
            for p in fields.iter_mut() {
                resolve_value(&mut p.value, pkg);
            }
        }
        PropertyValue::AtomicStruct(fields) => {
            for (_, v) in fields.iter_mut() {
                resolve_value(v, pkg);
            }
        }
        PropertyValue::Name(fname) => {
            *val = PropertyValue::String(pkg.fname_to_string(fname));
        }
        _ => {}
    }
}

impl Export {
    pub fn read(cursor: &mut Cursor<&Vec<u8>>, ver: i16) -> Result<Self> {
        let class_index = cursor.read_i32::<LittleEndian>()?;
        let super_index = cursor.read_i32::<LittleEndian>()?;
        let outer_index = cursor.read_i32::<LittleEndian>()?;

        let object_name = FName {
            name_index: cursor.read_i32::<LittleEndian>()?,
            name_instance: cursor.read_i32::<LittleEndian>()?,
        };

        let archetype = cursor.read_i32::<LittleEndian>()?;

        let object_flags = cursor.read_u64::<LittleEndian>()?;

        let serial_size = cursor.read_i32::<LittleEndian>()?;
        let serial_offset =
            if serial_size != 0 || ver >= VER_MOVED_EXPORTIMPORTMAPS_ADDED_TOTALHEADERSIZE {
                cursor.read_i32::<LittleEndian>()?
            } else {
                0
            };

        let mut legacy_component_map: HashMap<FName, i32> = HashMap::new();
        if ver < VER_REMOVED_COMPONENT_MAP {
            let count = cursor.read_i32::<LittleEndian>()?;
            for _ in 0..count {
                let k = FName {
                    name_index: cursor.read_i32::<LittleEndian>()?,
                    name_instance: cursor.read_i32::<LittleEndian>()?,
                };
                let v = cursor.read_i32::<LittleEndian>()?;
                legacy_component_map.insert(k, v);
            }
        }

        let export_flags = if ver >= VER_FOBJECTEXPORT_EXPORTFLAGS {
            cursor.read_u32::<LittleEndian>()?
        } else {
            0
        };

        let (generation_net_object_count, package_guid) = if ver >= VER_LINKERFREE_PACKAGEMAP {
            let gen_count = cursor.read_i32::<LittleEndian>()?;
            let mut gnoc = Vec::with_capacity(gen_count as usize);
            for _ in 0..gen_count {
                gnoc.push(cursor.read_i32::<LittleEndian>()?);
            }
            let guid = [
                cursor.read_i32::<LittleEndian>()?,
                cursor.read_i32::<LittleEndian>()?,
                cursor.read_i32::<LittleEndian>()?,
                cursor.read_i32::<LittleEndian>()?,
            ];
            (gnoc, guid)
        } else {
            (Vec::new(), [0; 4])
        };

        let package_flags = if ver >= VER_REMOVED_COMPONENT_MAP {
            cursor.read_u32::<LittleEndian>()?
        } else {
            0
        };

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
            package_flags,
        })
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Import {
    pub class_package: FName,
    pub class_name: FName,
    pub outer_index: i32,
    pub object_name: FName,
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
pub struct GenerationInfo {
    export_count: i32,
    name_count: i32,
    net_obj_count: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct FTextureType {
    pub size_x: i32,
    pub size_y: i32,
    pub num_mips: i32,
    pub format: u32,
    pub tex_create_flags: u32,
    pub export_indices: Vec<i32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct FTextureAllocations {
    pub texture_types: Vec<FTextureType>,
}

impl FTextureType {
    fn read<R: Read>(r: &mut R) -> Result<Self> {
        let size_x = r.read_i32::<LittleEndian>()?;
        let size_y = r.read_i32::<LittleEndian>()?;
        let num_mips = r.read_i32::<LittleEndian>()?;
        let format = r.read_u32::<LittleEndian>()?;
        let tex_create_flags = r.read_u32::<LittleEndian>()?;
        let n = r.read_i32::<LittleEndian>()?;
        if n < 0 || n > 0x10_0000 {
            return Err(Error::new(
                ErrorKind::InvalidData,
                format!("FTextureType: implausible ExportIndices count {}", n),
            ));
        }
        let mut export_indices = Vec::with_capacity(n as usize);
        for _ in 0..n {
            export_indices.push(r.read_i32::<LittleEndian>()?);
        }
        Ok(Self {
            size_x,
            size_y,
            num_mips,
            format,
            tex_create_flags,
            export_indices,
        })
    }

    fn write<W: Write>(&self, w: &mut W) -> Result<()> {
        w.write_i32::<LittleEndian>(self.size_x)?;
        w.write_i32::<LittleEndian>(self.size_y)?;
        w.write_i32::<LittleEndian>(self.num_mips)?;
        w.write_u32::<LittleEndian>(self.format)?;
        w.write_u32::<LittleEndian>(self.tex_create_flags)?;
        w.write_i32::<LittleEndian>(self.export_indices.len() as i32)?;
        for &i in &self.export_indices {
            w.write_i32::<LittleEndian>(i)?;
        }
        Ok(())
    }
}

impl FTextureAllocations {
    fn read<R: Read>(r: &mut R) -> Result<Self> {
        let n = r.read_i32::<LittleEndian>()?;
        if n < 0 || n > 0x10_0000 {
            return Err(Error::new(
                ErrorKind::InvalidData,
                format!("FTextureAllocations: implausible TextureTypes count {}", n),
            ));
        }
        let mut texture_types = Vec::with_capacity(n as usize);
        for _ in 0..n {
            texture_types.push(FTextureType::read(r)?);
        }
        Ok(Self { texture_types })
    }

    fn write<W: Write>(&self, w: &mut W) -> Result<()> {
        w.write_i32::<LittleEndian>(self.texture_types.len() as i32)?;
        for t in &self.texture_types {
            t.write(w)?;
        }
        Ok(())
    }
}

pub fn write_fstring<W: Write>(w: &mut W, s: &str) -> Result<()> {
    if s.is_empty() {
        w.write_i32::<LittleEndian>(0)?;
        return Ok(());
    }
    if s.chars().all(|c| (c as u32) <= 0xFF) {
        let bytes: Vec<u8> = s.chars().map(|c| c as u8).collect();
        let len = bytes.len() as i32 + 1;
        w.write_i32::<LittleEndian>(len)?;
        w.write_all(&bytes)?;
        w.write_u8(0)?;
    } else {
        let utf16: Vec<u16> = s.encode_utf16().collect();
        let len = -(utf16.len() as i32 + 1);
        w.write_i32::<LittleEndian>(len)?;
        for c in &utf16 {
            w.write_u16::<LittleEndian>(*c)?;
        }
        w.write_u16::<LittleEndian>(0)?;
    }
    Ok(())
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct UpkHeader {
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
    pub additional_packages: Vec<String>,
    pub texture_allocs: FTextureAllocations,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct UPKPak {
    pub name_table: Vec<String>,
    pub export_table: Vec<Export>,
    pub import_table: Vec<Import>,
}

impl UPKPak {
    pub fn parse_upk(cursor: &mut Cursor<&Vec<u8>>, header: &UpkHeader) -> Result<Self> {
        let name_count = header.name_count;
        let name_offset = header.name_offset;
        let export_count = header.export_count;
        let export_offset = header.export_offset;
        let import_count = header.import_count;
        let import_offset = header.import_offset;

        let mut name_table = Vec::new();
        cursor.set_position(name_offset as u64);
        for _ in 0..name_count {
            let name = read_name(cursor)?;
            name_table.push(name.name);
        }

        let mut export_table = Vec::new();
        cursor.set_position(export_offset as u64);
        for _ in 0..export_count {
            export_table.push(Export::read(cursor, header.p_ver)?);
        }

        let mut import_table = Vec::new();

        cursor.set_position(import_offset as u64);
        for _ in 0..import_count {
            import_table.push(Import::read(cursor)?);
        }

        Ok(Self {
            name_table,
            export_table,
            import_table,
        })
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
            let (object_name, outer_index, is_subobject) = if linker_index >= 0 {
                let idx = linker_index as usize;
                if let Some(import) = self.import_table.get(idx) {
                    let is_subobj = !result.is_empty()
                        && self.fname_to_string(&import.class_name) != "Package"
                        && self.is_package_outer(import.outer_index);

                    (
                        self.fname_to_string(&import.object_name),
                        import.outer_index,
                        is_subobj,
                    )
                } else {
                    break;
                }
            } else {
                let idx = (-linker_index - 1) as usize;
                if let Some(export) = self.export_table.get(idx) {
                    let is_subobj = !result.is_empty()
                        && self.get_class_name(-linker_index) != "Package"
                        && self.is_package_outer(export.outer_index);

                    (
                        self.fname_to_string(&export.object_name),
                        export.outer_index,
                        is_subobj,
                    )
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

                result = format!("{}{}", self.fname_to_string(&export.object_name), result);
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

    pub fn is_member_of_struct(&self, export_idx_1based: i32) -> bool {
        let mut cur = export_idx_1based;
        let mut guard = 0;
        loop {
            guard += 1;
            if guard > 64 {
                return false;
            }
            let exp = match self.export_table.get((cur - 1) as usize) {
                Some(e) => e,
                None => return false,
            };
            if exp.outer_index <= 0 {
                return false;
            }
            let outer = &self.export_table[(exp.outer_index - 1) as usize];
            let outer_class = self.get_class_name(outer.class_index);
            match outer_class.as_str() {
                "Class" | "State" | "ScriptStruct" | "Struct" | "Function" => return true,
                _ => {}
            }
            cur = exp.outer_index;
        }
    }
}

pub fn list_full_obj_paths(pkg: &UPKPak) -> Vec<String> {
    (0..pkg.export_table.len())
        .map(|idx| pkg.get_export_full_name((idx + 1) as i32))
        .collect()
}

fn read_tagged_at(
    db: &SchemaDb,
    owner_ref: &ResolvedRef,
    pkg: &UPKPak,
    p_ver: i16,
    target_idx: i32,
    offset: Option<u64>,
) -> Vec<crate::upkprops::Property> {
    db.open_package(&owner_ref.stem_lc)
        .ok()
        .and_then(|lp| lp.export_blob(target_idx).ok().map(|b| b.to_vec()))
        .and_then(|v| {
            let mut c = Cursor::new(&v);
            match offset {
                Some(off) => c.set_position(off),
                None => {
                    if p_ver >= VER_NETINDEX_STORED_AS_INT {
                        let _ = c.read_i32::<LittleEndian>();
                    }
                }
            }
            get_obj_props_with_db(&mut c, pkg, false, p_ver, Some(db), Some(owner_ref.clone()))
                .ok()
                .map(|(props, _)| props)
        })
        .unwrap_or_default()
}

fn render_meta_export(
    db: &SchemaDb,
    self_ref: &ResolvedRef,
    pkg: &UPKPak,
    pkg_stem: &str,
    p_ver: i16,
    export_index: i32,
    export_full_path: &str,
) -> Option<String> {
    use crate::schema::SchemaEntry;
    let entry = db.entry(self_ref).ok()?;
    match &*entry {
        SchemaEntry::ScriptStruct { extra, .. } => {
            let defaults = read_tagged_at(
                db,
                self_ref,
                pkg,
                p_ver,
                self_ref.export_idx,
                Some(extra.default_props_offset_in_blob),
            );
            crate::pseudo::render_struct_def(
                db,
                self_ref,
                pkg,
                pkg_stem,
                p_ver,
                export_index,
                export_full_path,
                &defaults,
            )
        }
        SchemaEntry::Struct { .. } => crate::pseudo::render_struct_def(
            db,
            self_ref,
            pkg,
            pkg_stem,
            p_ver,
            export_index,
            export_full_path,
            &[],
        ),
        SchemaEntry::Enum { .. } => crate::pseudo::render_enum_def(
            db,
            self_ref,
            pkg,
            pkg_stem,
            p_ver,
            export_index,
            export_full_path,
        ),
        SchemaEntry::Const { .. } => crate::pseudo::render_const_def(
            db,
            self_ref,
            pkg,
            pkg_stem,
            p_ver,
            export_index,
            export_full_path,
        ),
        SchemaEntry::Property(_) => crate::pseudo::render_property_def(
            db,
            self_ref,
            pkg,
            pkg_stem,
            p_ver,
            export_index,
            export_full_path,
        ),
        _ => None,
    }
}

pub fn write_extracted_file(
    path: &Path,
    buf: &[u8],
    pkg: &UPKPak,
    pkg_stem: &str,
    p_ver: i16,
    db: Option<&SchemaDb>,
    owner_class_ref: Option<ResolvedRef>,
    self_ref: Option<ResolvedRef>,
    export_index: i32,
    export_full_path: &str,
    registry: &NativeRegistry,
) -> Result<PathBuf> {
    let name = path.file_stem().and_then(|s| s.to_str()).unwrap_or("obj");
    let ext = path.extension().and_then(|s| s.to_str()).unwrap_or("bin");
    let dir = path.parent().unwrap();
    std::fs::create_dir_all(dir)?;

    if ext == "Class" {
        if let (Some(db), Some(self_ref)) = (db, self_ref.as_ref()) {
            let cdo_props = match db.entry(self_ref) {
                Ok(e) => match &*e {
                    crate::schema::SchemaEntry::Class { extra, .. }
                        if extra.class_default_object > 0 =>
                    {
                        let cdo_idx = extra.class_default_object;
                        db.open_package(&self_ref.stem_lc)
                            .ok()
                            .and_then(|lp| lp.export_blob(cdo_idx).ok().map(|b| b.to_vec()))
                            .and_then(|v| {
                                let mut c = Cursor::new(&v);
                                if p_ver >= VER_NETINDEX_STORED_AS_INT {
                                    let _ = c.read_i32::<LittleEndian>();
                                }
                                get_obj_props_with_db(
                                    &mut c,
                                    pkg,
                                    false,
                                    p_ver,
                                    Some(db),
                                    Some(self_ref.clone()),
                                )
                                .ok()
                                .map(|(props, _)| props)
                            })
                            .unwrap_or_default()
                    }
                    _ => Vec::new(),
                },
                Err(_) => Vec::new(),
            };

            if let Some(text) = crate::pseudo::render_class_def(
                db,
                self_ref,
                pkg,
                pkg_stem,
                p_ver,
                export_index,
                export_full_path,
                &cdo_props,
            ) {
                let uo_path = dir.join(format!("{name}.uo"));
                std::fs::write(&uo_path, text.as_bytes())?;
                return Ok(uo_path);
            }
        }
    }

    let is_meta_def =
        matches!(ext, "ScriptStruct" | "Struct" | "Enum" | "Const") || ext.ends_with("Property");
    if is_meta_def {
        if let (Some(db), Some(self_ref)) = (db, self_ref.as_ref()) {
            if let Some(text) = render_meta_export(
                db,
                self_ref,
                pkg,
                pkg_stem,
                p_ver,
                export_index,
                export_full_path,
            ) {
                let uo_path = dir.join(format!("{name}.uo"));
                std::fs::write(&uo_path, text.as_bytes())?;
                return Ok(uo_path);
            }
        }
    }

    let buf_vec = buf.to_vec();
    let mut cursor = Cursor::new(&buf_vec);

    let net_index = if p_ver >= VER_NETINDEX_STORED_AS_INT {
        Some(cursor.read_i32::<LittleEndian>()?)
    } else {
        None
    };

    let (props, props_end) =
        get_obj_props_with_db(&mut cursor, pkg, false, p_ver, db, owner_class_ref.clone())?;

    let tail = &buf_vec[props_end as usize..];

    let ser = registry.for_class(db, owner_class_ref.as_ref(), ext);
    let read = match &ser {
        Some(s) => s.read(&NativeReadCtx {
            blob: tail,
            props: &props,
            ver: p_ver,
            l_ver: 0,
            pak: pkg,
            db,
            self_ref: self_ref.clone(),
            class_ref: owner_class_ref.clone(),
        })?,
        None => {
            if tail.is_empty() {
                NativeRead::just(NativePayload::Empty { tail: Vec::new() })
            } else if let (Some(db), Some(cref)) = (db, owner_class_ref.as_ref()) {
                match crate::upkprops::read_native_props(tail, pkg, p_ver, db, cref, &props) {
                    Some(fields) => NativeRead::just(NativePayload::NativeProps { fields }),
                    None => NativeRead::just(NativePayload::Raw {
                        bytes: tail.to_vec(),
                    }),
                }
            } else {
                NativeRead::just(NativePayload::Raw {
                    bytes: tail.to_vec(),
                })
            }
        }
    };

    let sidecars = match &ser {
        Some(s) => s.emit_external(&read.payload, dir, name)?,
        None => Vec::new(),
    };

    let uo_path = dir.join(format!("{name}.uo"));
    crate::pseudo::write_uo_file(
        &uo_path,
        &EmitInput {
            class_name: ext,
            export_short_name: name,
            export_full_path,
            export_index,
            net_index,
            props: &props,
            consumed_props: &read.consumed_props,
            payload: &read.payload,
            sidecars: &sidecars,
            pak: pkg,
            pkg_stem,
            p_ver,
        },
    )?;

    Ok(uo_path)
}

pub fn extract_by_name(
    cursor: &mut Cursor<Vec<u8>>,
    pkg: &UPKPak,
    path: &str,
    out_dir: &Path,
    all: bool,
    ver: i16,
    db: Option<&SchemaDb>,
    pkg_stem_lc: &str,
) -> Result<()> {
    let registry = NativeRegistry::standard();
    let mut found = false;

    for (idx, exp) in pkg.export_table.iter().enumerate() {
        let export_idx_1 = (idx + 1) as i32;
        let full_name = pkg.get_export_full_name(export_idx_1);
        let fs_path = UPKPak::ue_name_to_path(&full_name);

        if !(fs_path.contains(path) || full_name.contains(path) || all) {
            continue;
        }

        let file_path = out_dir.join(&fs_path);
        if let Some(parent) = file_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        cursor.seek(std::io::SeekFrom::Start(exp.serial_offset as u64))?;
        let mut buffer = vec![0u8; exp.serial_size as usize];
        cursor.read_exact(&mut buffer)?;

        let class_ref = if exp.class_index > 0 {
            Some(ResolvedRef {
                stem_lc: pkg_stem_lc.into(),
                export_idx: exp.class_index,
            })
        } else if exp.class_index < 0 {
            db.and_then(|d| {
                d.open_package(pkg_stem_lc)
                    .ok()
                    .and_then(|lp| d.resolve_index(&lp, exp.class_index).ok().flatten())
            })
        } else {
            None
        };

        let self_ref = Some(ResolvedRef {
            stem_lc: pkg_stem_lc.into(),
            export_idx: export_idx_1,
        });

        let out_path = write_extracted_file(
            &file_path,
            &buffer,
            pkg,
            pkg_stem_lc,
            ver,
            db,
            class_ref,
            self_ref,
            export_idx_1,
            &full_name,
            &registry,
        )?;

        println!(
            "Exported \x1b[93m{}\x1b[0m (\x1b[33m{}\x1b[0m bytes) → \x1b[32m{}\x1b[0m",
            full_name,
            buffer.len(),
            out_path.display()
        );
        found = true;
    }
    if !found {
        println!("File {path} not exists in package.");
    }
    Ok(())
}

pub fn read_name(cursor: &mut Cursor<&Vec<u8>>) -> Result<NameEntry> {
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

pub fn read_string(cursor: &mut Cursor<&Vec<u8>>) -> Result<String> {
    let len = cursor.read_i32::<LittleEndian>()?;
    if len == 0 {
        return Ok("".to_string());
    }

    let remaining = cursor
        .get_ref()
        .len()
        .saturating_sub(cursor.position() as usize) as u64;
    let needed: u64 = if len > 0 {
        len as u64
    } else {
        (len as i64).unsigned_abs().saturating_mul(2)
    };
    if needed > remaining {
        return Err(Error::new(
            ErrorKind::InvalidData,
            format!("FString length {len} exceeds {remaining} remaining bytes"),
        ));
    }

    if len > 0 {
        let mut buf = vec![0u8; len as usize];
        cursor.read_exact(&mut buf)?;

        if buf.last() == Some(&0) {
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

        let utf16_trimmed = match utf16.last() {
            Some(&0) => &utf16[..utf16.len() - 1],
            _ => &utf16[..],
        };

        String::from_utf16(utf16_trimmed)
            .map_err(|_| Error::new(ErrorKind::InvalidData, "Invalid UTF16"))
    }
}

pub fn read_fstring_stream<R: Read>(r: &mut R) -> Result<String> {
    let len = r.read_i32::<LittleEndian>()?;
    if len == 0 {
        return Ok(String::new());
    }
    if len > 0 {
        if len > 0x10_0000 {
            return Err(Error::new(
                ErrorKind::InvalidData,
                format!("FString: implausible ANSI length {}", len),
            ));
        }
        let mut buf = vec![0u8; len as usize];
        r.read_exact(&mut buf)?;
        if buf.last() == Some(&0) {
            buf.pop();
        }
        Ok(buf.iter().map(|&b| b as char).collect())
    } else {
        let n = (-len) as usize;
        if n > 0x10_0000 {
            return Err(Error::new(
                ErrorKind::InvalidData,
                format!("FString: implausible UTF-16 length {}", n),
            ));
        }
        let mut u = vec![0u16; n];
        for slot in &mut u {
            *slot = r.read_u16::<LittleEndian>()?;
        }
        if u.last() == Some(&0) {
            u.pop();
        }
        String::from_utf16(&u).map_err(|_| Error::new(ErrorKind::InvalidData, "bad UTF-16"))
    }
}

pub fn get_obj_props(
    cursor: &mut Cursor<&Vec<u8>>,
    upk: &UPKPak,
    print_out: bool,
    ver: i16,
) -> Result<(Vec<Property>, u64)> {
    let mut props = Vec::new();
    let mut last_pos = cursor.position();

    loop {
        let before = cursor.position();

        let result = upkprops::parse_property(cursor, upk, ver);

        match result {
            Err(_) => {
                cursor.set_position(last_pos);
                break;
            }
            Ok(None) => {
                cursor.set_position(before);
                break;
            }
            Ok(Some(prop)) => {
                if print_out {
                    println!("{:?}", prop);
                }
                last_pos = cursor.position();
                let is_none = prop.name == "None";
                props.push(prop);
                if is_none {
                    break;
                }
            }
        }
    }

    Ok((props, cursor.position()))
}

pub fn get_obj_props_with_db(
    cursor: &mut Cursor<&Vec<u8>>,
    upk: &UPKPak,
    print_out: bool,
    ver: i16,
    db: Option<&SchemaDb>,
    owner: Option<ResolvedRef>,
) -> Result<(Vec<Property>, u64)> {
    let ctx = PropertyCtx {
        pak: upk,
        ver,
        db,
        owner,
    };
    let mut props = Vec::new();
    let mut last_pos = cursor.position();
    loop {
        let before = cursor.position();
        let result = parse_property_ctx(cursor, &ctx);
        match result {
            Err(_) => {
                cursor.set_position(last_pos);
                break;
            }
            Ok(None) => {
                cursor.set_position(before);
                break;
            }
            Ok(Some(prop)) => {
                if print_out {
                    println!("{:?}", prop);
                }
                last_pos = cursor.position();
                let is_none = prop.name == "None";
                props.push(prop);
                if is_none {
                    break;
                }
            }
        }
    }
    Ok((props, cursor.position()))
}
pub fn get_obj_props_with_netindex(
    cursor: &mut Cursor<&Vec<u8>>,
    upk: &UPKPak,
    print_out: bool,
    ver: i16,
    db: Option<&SchemaDb>,
    owner: Option<ResolvedRef>,
) -> Result<(Vec<Property>, u64)> {
    let _net = cursor.read_i32::<LittleEndian>()?;
    get_obj_props_with_db(cursor, upk, print_out, ver, db, owner)
}

impl fmt::Display for UpkHeader {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
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
        if self.p_ver >= VER_ADDED_CROSSLEVEL_REFERENCES {
            writeln!(
                f,
                "Import/Export Guids pos: {}",
                self.import_export_guids_offset
            )?;
            writeln!(f, "Import Guids Count: {}", self.import_guids_count)?;
            writeln!(f, "Export Guids Count: {}", self.export_guids_count)?;
        }
        if self.p_ver >= VER_ASSET_THUMBNAILS_IN_PACKAGES {
            writeln!(f, "Thumbnail table pos: {}", self.thumbnail_table_offest)?;
        }
        writeln!(f, "GUID: {:x?}", self.guid)?;
        if self.gen_count > 0 {
            writeln!(f, "Generations (Count={}):", self.gen_count)?;
        }
        for (i, gens) in self.gens.iter().enumerate() {
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
            writeln!(
                f,
                "Num of compressed chunks: {}",
                self.compressed_chunks_count
            )?;
            for (i, c) in self.compressed_chunks.iter().enumerate() {
                writeln!(
                    f,
                    " - Chunk {}:\n\
                    \x20  * Decompressed offset = {}\n\
                    \x20  * Decompressed size   = {}\n\
                    \x20  * Compressed offset   = {}\n\
                    \x20  * Compressed size     = {}",
                    i,
                    c.decompressed_offset,
                    c.decompressed_size,
                    c.compressed_offset,
                    c.compressed_size
                )?;
            }
        }

        writeln!(f, "Package Source: {}", self.package_source)?;

        if self.p_ver >= VER_ADDITIONAL_COOK_PACKAGE_SUMMARY {
            writeln!(
                f,
                "Additional packages to cook: {}",
                self.additional_packages.len()
            )?;
            for (i, p) in self.additional_packages.iter().enumerate() {
                writeln!(f, " - [{}] {}", i, p)?;
            }
        }

        if self.p_ver >= VER_TEXTURE_PREALLOCATION {
            writeln!(
                f,
                "TextureAllocations: {} type(s)",
                self.texture_allocs.texture_types.len()
            )?;
            for (i, t) in self.texture_allocs.texture_types.iter().enumerate() {
                writeln!(
                    f,
                    " - [{}] {}x{} mips={} fmt={} flags=0x{:08x} -> {} export(s)",
                    i,
                    t.size_x,
                    t.size_y,
                    t.num_mips,
                    t.format,
                    t.tex_create_flags,
                    t.export_indices.len()
                )?;
            }
        }

        Ok(())
    }
}

impl UpkHeader {
    pub fn read<R: Read + Seek>(mut reader: R) -> Result<Self> {
        let sign = reader.read_u32::<LittleEndian>()?;
        if sign != PACKAGE_FILE_TAG {
            return Err(Error::new(
                ErrorKind::InvalidData,
                format!("Invalid file signature, sig=0x{:X}", sign),
            ));
        }

        let p_ver = reader.read_i16::<LittleEndian>()?;
        let l_ver = reader.read_i16::<LittleEndian>()?;
        let header_size = reader.read_i32::<LittleEndian>()?;

        let path_len = reader.read_i32::<LittleEndian>()?;
        let mut rfl = path_len;
        if path_len < 0 {
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
        let depends_offset = if p_ver >= VER_ADDED_LINKER_DEPENDENCIES {
            reader.read_i32::<LittleEndian>()?
        } else {
            0
        };

        if import_count <= 0 || name_count <= 0 || export_count <= 0 {
            return Err(Error::new(ErrorKind::InvalidData, "Corrupted pak"));
        }

        let mut import_export_guids_offset = -1;
        let mut import_guids_count = 0;
        let mut export_guids_count = 0;
        let mut thumbnail_table_offest = 0;

        if p_ver >= VER_ADDED_CROSSLEVEL_REFERENCES {
            import_export_guids_offset = reader.read_i32::<LittleEndian>()?;
            import_guids_count = reader.read_u32::<LittleEndian>()?;
            export_guids_count = reader.read_u32::<LittleEndian>()?;
        }

        if p_ver >= VER_ASSET_THUMBNAILS_IN_PACKAGES {
            thumbnail_table_offest = reader.read_u32::<LittleEndian>()?;
        }

        let guid = [
            reader.read_i32::<LittleEndian>()?,
            reader.read_i32::<LittleEndian>()?,
            reader.read_i32::<LittleEndian>()?,
            reader.read_i32::<LittleEndian>()?,
        ];

        let gen_count = reader.read_i32::<LittleEndian>()?;
        let mut gens = Vec::with_capacity(gen_count as usize);

        for _ in 0..gen_count {
            let export_count = reader.read_i32::<LittleEndian>()?;
            let name_count = reader.read_i32::<LittleEndian>()?;
            let net_obj_count = if p_ver >= VER_LINKERFREE_PACKAGEMAP {
                reader.read_i32::<LittleEndian>()?
            } else {
                0
            };
            gens.push(GenerationInfo {
                export_count,
                name_count,
                net_obj_count,
            });
        }

        let engine_ver = if p_ver >= VER_PACKAGEFILESUMMARY_CHANGE {
            reader.read_i32::<LittleEndian>()?
        } else {
            0
        };
        let cooker_ver = if p_ver >= VER_PACKAGEFILESUMMARY_CHANGE_COOK_VER_ADDED {
            reader.read_i32::<LittleEndian>()?
        } else {
            0
        };

        let (compression_method, compressed_chunks_count, compressed_chunks) =
            if p_ver >= VER_ADDED_PACKAGE_COMPRESSION_SUPPORT {
                let m = CompressionMethod::try_from(reader.read_u32::<LittleEndian>()?).unwrap();
                let n = reader.read_u32::<LittleEndian>()?;
                let mut v: Vec<CompressedChunk> = Vec::with_capacity(n as usize);
                for _ in 0..n {
                    v.push(CompressedChunk {
                        decompressed_offset: reader.read_u32::<LittleEndian>()?,
                        decompressed_size: reader.read_u32::<LittleEndian>()?,
                        compressed_offset: reader.read_u32::<LittleEndian>()?,
                        compressed_size: reader.read_u32::<LittleEndian>()?,
                    });
                }
                (m, n, v)
            } else {
                (CompressionMethod::None, 0, Vec::new())
            };

        let package_source = if p_ver >= VER_ADDED_PACKAGE_COMPRESSION_SUPPORT {
            reader.read_i32::<LittleEndian>()?
        } else {
            0
        };

        let additional_packages = if p_ver >= VER_ADDITIONAL_COOK_PACKAGE_SUMMARY {
            let n = reader.read_i32::<LittleEndian>()?;
            let mut v = Vec::with_capacity(n as usize);
            for _ in 0..n {
                v.push(read_fstring_stream(&mut reader)?);
            }
            v
        } else {
            Vec::new()
        };

        let texture_allocs = if p_ver >= VER_TEXTURE_PREALLOCATION {
            FTextureAllocations::read(&mut reader)?
        } else {
            FTextureAllocations::default()
        };

        let header = UpkHeader {
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
            texture_allocs,
        };

        Ok(header)
    }

    pub fn write<R: Write + Seek>(&self, mut writer: R) -> Result<()> {
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
        if self.p_ver >= VER_ADDED_LINKER_DEPENDENCIES {
            writer.write_i32::<LittleEndian>(self.depends_offset)?;
        }

        if self.p_ver >= VER_ADDED_CROSSLEVEL_REFERENCES {
            writer.write_i32::<LittleEndian>(self.import_export_guids_offset)?;
            writer.write_u32::<LittleEndian>(self.import_guids_count)?;
            writer.write_u32::<LittleEndian>(self.export_guids_count)?;
        }
        if self.p_ver >= VER_ASSET_THUMBNAILS_IN_PACKAGES {
            writer.write_u32::<LittleEndian>(self.thumbnail_table_offest)?;
        }

        for v in &self.guid {
            writer.write_i32::<LittleEndian>(*v)?;
        }

        writer.write_i32::<LittleEndian>(self.gens.len() as i32)?;

        for g in &self.gens {
            writer.write_i32::<LittleEndian>(g.export_count)?;
            writer.write_i32::<LittleEndian>(g.name_count)?;
            if self.p_ver >= VER_LINKERFREE_PACKAGEMAP {
                writer.write_i32::<LittleEndian>(g.net_obj_count)?;
            }
        }

        if self.p_ver >= VER_PACKAGEFILESUMMARY_CHANGE {
            writer.write_i32::<LittleEndian>(self.engine_ver)?;
        }
        if self.p_ver >= VER_PACKAGEFILESUMMARY_CHANGE_COOK_VER_ADDED {
            writer.write_i32::<LittleEndian>(self.cooker_ver)?;
        }

        if self.p_ver >= VER_ADDED_PACKAGE_COMPRESSION_SUPPORT {
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
        }

        if self.p_ver >= VER_ADDED_PACKAGE_COMPRESSION_SUPPORT {
            writer.write_i32::<LittleEndian>(self.package_source)?;
        }

        if self.p_ver >= VER_ADDITIONAL_COOK_PACKAGE_SUMMARY {
            writer.write_i32::<LittleEndian>(self.additional_packages.len() as i32)?;
            for s in &self.additional_packages {
                write_fstring(&mut writer, s)?;
            }
        }

        if self.p_ver >= VER_TEXTURE_PREALLOCATION {
            self.texture_allocs.write(&mut writer)?;
        }

        Ok(())
    }

    pub fn has_flag(&self, flag: u32) -> bool {
        (self.pak_flags & flag) != 0
    }
}
