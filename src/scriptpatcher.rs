use std::io::{Cursor, Write};
use byteorder::{LittleEndian, WriteBytesExt};

#[derive(Debug)]
pub struct ScriptPatchData {
    pub struct_name: String,
    pub patch: PatchData,
}

#[derive(Debug)]
pub struct EnumPatchData {
    pub enum_name: String,
    pub enum_path_name: String,
    pub enum_values: Vec<String>,
}

#[derive(Debug)]
pub struct ObjectExportPatch {
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
    pub package_guid: [i32; 4],
    pub package_flags: u32,
}

#[derive(Debug)]
pub struct ObjectImportPatch {
    pub class_package: String,
    pub class_name: String,
    pub outer_index: i32,
    pub object_name: String,
}

#[derive(Debug)]
pub struct PatchData {
    pub package_name: String,
    pub names: Vec<String>,
    pub exports: Vec<ObjectExportPatch>,
    pub imports: Vec<ObjectImportPatch>,
    pub new_objects: Vec<PatchData>,
    pub modified_class_default_objects: Vec<PatchData>,
    pub modified_enums: Vec<EnumPatchData>,
    pub script_patches: Vec<ScriptPatchData>,
}
