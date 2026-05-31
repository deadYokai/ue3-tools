use std::io::{Cursor, Error, ErrorKind, Read, Result, Seek, SeekFrom};

use byteorder::{LittleEndian, ReadBytesExt};
use serde::{Deserialize, Serialize};

use crate::upkprops::parse_property;
use crate::upkreader::{FName, UPKPak, read_string};
use crate::versions::*;

#[derive(Debug, Clone, Copy)]
pub struct SchemaParseCtx {
    pub p_ver: i16,
    pub strip_editor_only: bool,
}

impl SchemaParseCtx {
    pub fn from_package(pak: &UPKPak, p_ver: i16, strip_editor_only: bool) -> Self {
        let _ = pak;
        Self {
            p_ver,
            strip_editor_only,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PropertyCommon {
    pub next: i32,
    pub super_field: Option<i32>,
    pub array_dim: i32,
    pub property_flags: u64,
    pub category: Option<FName>,
    pub array_size_enum: Option<i32>,
    pub rep_offset: Option<u16>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PropertyKind {
    Byte {
        common: PropertyCommon,
        enum_obj: i32,
    },
    Int {
        common: PropertyCommon,
    },
    Bool {
        common: PropertyCommon,
    },
    Float {
        common: PropertyCommon,
    },
    Object {
        common: PropertyCommon,
        property_class: i32,
    },
    Class {
        common: PropertyCommon,
        property_class: i32,
        meta_class: i32,
    },
    Component {
        common: PropertyCommon,
        property_class: i32,
    },
    Interface {
        common: PropertyCommon,
        interface_class: i32,
    },
    Name {
        common: PropertyCommon,
    },
    Str {
        common: PropertyCommon,
    },
    Delegate {
        common: PropertyCommon,
        function: i32,
        source_delegate: FName,
    },
    Array {
        common: PropertyCommon,
        inner: i32,
    },
    Map {
        common: PropertyCommon,
        key: i32,
        value: i32,
    },
    Struct {
        common: PropertyCommon,
        struct_obj: i32,
    },
}

impl PropertyKind {
    pub fn common(&self) -> &PropertyCommon {
        match self {
            PropertyKind::Byte { common, .. } => common,
            PropertyKind::Int { common } => common,
            PropertyKind::Bool { common } => common,
            PropertyKind::Float { common } => common,
            PropertyKind::Object { common, .. } => common,
            PropertyKind::Class { common, .. } => common,
            PropertyKind::Component { common, .. } => common,
            PropertyKind::Interface { common, .. } => common,
            PropertyKind::Name { common } => common,
            PropertyKind::Str { common } => common,
            PropertyKind::Delegate { common, .. } => common,
            PropertyKind::Array { common, .. } => common,
            PropertyKind::Map { common, .. } => common,
            PropertyKind::Struct { common, .. } => common,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StructHeader {
    pub next: i32,
    pub super_struct: i32,
    pub script_text: Option<i32>,
    pub children: i32,
    pub cpp_text: Option<i32>,
    pub editor_line_pos: Option<(i32, i32)>,
    pub bytecode_size: i32,
    pub on_disk_script_size: i32,
    pub script_offset_in_blob: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SchemaEntry {
    Class {
        header: StructHeader,
        class_flags: u32,
        class_within: i32,
        class_config_name: FName,
        component_name_to_default_object_map: Vec<(FName, i32)>,
        interfaces: Vec<(i32, i32)>,
        dont_sort_categories: Option<Vec<FName>>,
        hide_categories: Option<Vec<FName>>,
        auto_expand_categories: Option<Vec<FName>>,
        auto_collapse_categories: Option<Vec<FName>>,
        force_script_order: Option<bool>,
        class_group_names: Option<Vec<FName>>,
        class_header_filename: Option<String>,
        dll_bind_name: Option<FName>,
        class_default_object: i32,
    },
    State {
        header: StructHeader,
        probe_mask: u64,
        label_table_offset: u16,
        state_flags: u32,
        func_map: Vec<(FName, i32)>,
    },
    ScriptStruct {
        header: StructHeader,
        struct_flags: u32,
    },
    Struct {
        header: StructHeader,
    },
    Function {
        header: StructHeader,
        i_native: u16,
        oper_precedence: u8,
        function_flags: u32,
        rep_offset: Option<u16>,
        friendly_name: Option<FName>,
    },
    Enum {
        next: i32,
        super_field: Option<i32>,
        names: Vec<FName>,
    },
    Property(PropertyKind),
}

pub fn parse_export_schema(
    blob: &[u8],
    class_name: &str,
    pak: &UPKPak,
    ctx: SchemaParseCtx,
) -> Result<Option<SchemaEntry>> {
    let v = blob.to_vec();
    let mut c = Cursor::new(&v);

    skip_object_prefix(&mut c, pak, ctx.p_ver)?;

    match class_name {
        "Class" => Ok(Some(parse_class(&mut c, pak, ctx)?)),
        "State" => Ok(Some(parse_state(&mut c, pak, ctx)?)),
        "ScriptStruct" => Ok(Some(parse_script_struct(&mut c, pak, ctx)?)),
        "Struct" => Ok(Some(SchemaEntry::Struct {
            header: parse_struct_header(&mut c, pak, ctx)?,
        })),
        "Function" => Ok(Some(parse_function(&mut c, pak, ctx)?)),
        "Enum" => Ok(Some(parse_enum(&mut c, pak, ctx)?)),

        "ByteProperty" => Ok(Some(SchemaEntry::Property(parse_byte_property(
            &mut c, ctx,
        )?))),
        "IntProperty" => Ok(Some(SchemaEntry::Property(parse_int_property(
            &mut c, ctx,
        )?))),
        "BoolProperty" => Ok(Some(SchemaEntry::Property(parse_bool_property(
            &mut c, ctx,
        )?))),
        "FloatProperty" => Ok(Some(SchemaEntry::Property(parse_float_property(
            &mut c, ctx,
        )?))),
        "ObjectProperty" => Ok(Some(SchemaEntry::Property(parse_object_property(
            &mut c, ctx,
        )?))),
        "ClassProperty" => Ok(Some(SchemaEntry::Property(parse_class_property(
            &mut c, ctx,
        )?))),
        "ComponentProperty" => Ok(Some(SchemaEntry::Property(parse_component_property(
            &mut c, ctx,
        )?))),
        "InterfaceProperty" => Ok(Some(SchemaEntry::Property(parse_interface_property(
            &mut c, ctx,
        )?))),
        "NameProperty" => Ok(Some(SchemaEntry::Property(parse_name_property(
            &mut c, ctx,
        )?))),
        "StrProperty" => Ok(Some(SchemaEntry::Property(parse_str_property(
            &mut c, ctx,
        )?))),
        "DelegateProperty" => Ok(Some(SchemaEntry::Property(parse_delegate_property(
            &mut c, ctx,
        )?))),
        "ArrayProperty" => Ok(Some(SchemaEntry::Property(parse_array_property(
            &mut c, ctx,
        )?))),
        "MapProperty" => Ok(Some(SchemaEntry::Property(parse_map_property(
            &mut c, ctx,
        )?))),
        "StructProperty" => Ok(Some(SchemaEntry::Property(parse_struct_property(
            &mut c, ctx,
        )?))),

        _ => Ok(None),
    }
}

fn skip_object_prefix(c: &mut Cursor<&Vec<u8>>, pak: &UPKPak, p_ver: i16) -> Result<()> {
    let _net_index = c.read_i32::<LittleEndian>()?;

    loop {
        let before = c.position();
        match parse_property(c, pak, p_ver) {
            Ok(Some(p)) if p.name == "None" => return Ok(()),
            Ok(Some(_)) => continue,
            Ok(None) => {
                c.set_position(before);
                return Ok(());
            }
            Err(e) => return Err(e),
        }
    }
}

fn parse_field_prefix(c: &mut Cursor<&Vec<u8>>, p_ver: i16) -> Result<(i32, Option<i32>)> {
    let pre_756_super = if p_ver < VER_MOVED_SUPERFIELD_TO_USTRUCT {
        Some(c.read_i32::<LittleEndian>()?)
    } else {
        None
    };
    let next = c.read_i32::<LittleEndian>()?;
    Ok((next, pre_756_super))
}

fn parse_struct_header(
    c: &mut Cursor<&Vec<u8>>,
    pak: &UPKPak,
    ctx: SchemaParseCtx,
) -> Result<StructHeader> {
    let (next, pre_756_super) = parse_field_prefix(c, ctx.p_ver)?;

    let super_struct = if ctx.p_ver >= VER_MOVED_SUPERFIELD_TO_USTRUCT {
        c.read_i32::<LittleEndian>()?
    } else {
        pre_756_super.unwrap_or(0)
    };

    let script_text = if !ctx.strip_editor_only {
        Some(c.read_i32::<LittleEndian>()?)
    } else {
        None
    };

    let children = c.read_i32::<LittleEndian>()?;

    let (cpp_text, editor_line_pos) = if !ctx.strip_editor_only {
        let cpp = c.read_i32::<LittleEndian>()?;
        let line = c.read_i32::<LittleEndian>()?;
        let tpos = c.read_i32::<LittleEndian>()?;
        (Some(cpp), Some((line, tpos)))
    } else {
        (None, None)
    };

    let bytecode_size = c.read_i32::<LittleEndian>()?;
    let on_disk_script_size = if ctx.p_ver >= VER_USTRUCT_SERIALIZE_ONDISK_SCRIPTSIZE {
        c.read_i32::<LittleEndian>()?
    } else {
        bytecode_size
    };

    let script_offset_in_blob = c.position();
    if on_disk_script_size > 0 {
        c.seek(SeekFrom::Current(on_disk_script_size as i64))?;
    }

    let _ = pak;
    Ok(StructHeader {
        next,
        super_struct,
        script_text,
        children,
        cpp_text,
        editor_line_pos,
        bytecode_size,
        on_disk_script_size,
        script_offset_in_blob,
    })
}

fn parse_script_struct(
    c: &mut Cursor<&Vec<u8>>,
    pak: &UPKPak,
    ctx: SchemaParseCtx,
) -> Result<SchemaEntry> {
    let header = parse_struct_header(c, pak, ctx)?;
    let struct_flags = c.read_u32::<LittleEndian>()?;
    Ok(SchemaEntry::ScriptStruct {
        header,
        struct_flags,
    })
}

fn parse_state(c: &mut Cursor<&Vec<u8>>, pak: &UPKPak, ctx: SchemaParseCtx) -> Result<SchemaEntry> {
    let header = parse_struct_header(c, pak, ctx)?;
    let probe_mask = c.read_u64::<LittleEndian>()?;
    let label_table_offset = c.read_u16::<LittleEndian>()?;
    let state_flags = c.read_u32::<LittleEndian>()?;
    let func_map = read_fname_to_object_map(c)?;
    Ok(SchemaEntry::State {
        header,
        probe_mask,
        label_table_offset,
        state_flags,
        func_map,
    })
}

fn parse_class(c: &mut Cursor<&Vec<u8>>, pak: &UPKPak, ctx: SchemaParseCtx) -> Result<SchemaEntry> {
    let SchemaEntry::State {
        header,
        probe_mask: _,
        label_table_offset: _,
        state_flags: _,
        func_map: _,
    } = parse_state(c, pak, ctx)?
    else {
        unreachable!()
    };

    let class_flags = c.read_u32::<LittleEndian>()?;
    let class_within = c.read_i32::<LittleEndian>()?;
    let class_config_name = read_fname(c)?;
    let component_name_to_default_object_map = read_fname_to_object_map(c)?;
    let interfaces = read_object_to_object_map(c)?;

    let mut dont_sort_categories = None;
    let mut hide_categories = None;
    let mut auto_expand_categories = None;
    let mut auto_collapse_categories = None;
    let mut force_script_order = None;
    let mut class_group_names = None;
    let mut class_header_filename = None;

    if !ctx.strip_editor_only {
        if ctx.p_ver >= 603 {
            dont_sort_categories = Some(read_fname_array(c)?);
        }
        hide_categories = Some(read_fname_array(c)?);
        auto_expand_categories = Some(read_fname_array(c)?);
        auto_collapse_categories = Some(read_fname_array(c)?);

        if ctx.p_ver >= 749 {
            force_script_order = Some(c.read_i32::<LittleEndian>()? != 0);
        }

        if ctx.p_ver >= 789 {
            class_group_names = Some(read_fname_array(c)?);
        }

        class_header_filename = Some(read_string(c)?);
    }

    let dll_bind_name = if ctx.p_ver >= VER_SCRIPT_BIND_DLL_FUNCTIONS {
        Some(read_fname(c)?)
    } else {
        None
    };

    let class_default_object = c.read_i32::<LittleEndian>()?;

    Ok(SchemaEntry::Class {
        header,
        class_flags,
        class_within,
        class_config_name,
        component_name_to_default_object_map,
        interfaces,
        dont_sort_categories,
        hide_categories,
        auto_expand_categories,
        auto_collapse_categories,
        force_script_order,
        class_group_names,
        class_header_filename,
        dll_bind_name,
        class_default_object,
    })
}

fn parse_function(
    c: &mut Cursor<&Vec<u8>>,
    pak: &UPKPak,
    ctx: SchemaParseCtx,
) -> Result<SchemaEntry> {
    let header = parse_struct_header(c, pak, ctx)?;

    let i_native = c.read_u16::<LittleEndian>()?;
    let oper_precedence = c.read_u8()?;
    let function_flags = c.read_u32::<LittleEndian>()?;
    let rep_offset = if function_flags & FUNC_NET != 0 {
        Some(c.read_u16::<LittleEndian>()?)
    } else {
        None
    };
    let friendly_name = if !ctx.strip_editor_only {
        Some(read_fname(c)?)
    } else {
        None
    };

    Ok(SchemaEntry::Function {
        header,
        i_native,
        oper_precedence,
        function_flags,
        rep_offset,
        friendly_name,
    })
}

fn parse_enum(c: &mut Cursor<&Vec<u8>>, _pak: &UPKPak, ctx: SchemaParseCtx) -> Result<SchemaEntry> {
    let (next, super_field) = parse_field_prefix(c, ctx.p_ver)?;
    let names = read_fname_array(c)?;
    Ok(SchemaEntry::Enum {
        next,
        super_field,
        names,
    })
}

fn parse_property_common(c: &mut Cursor<&Vec<u8>>, ctx: SchemaParseCtx) -> Result<PropertyCommon> {
    let (next, pre_756_super) = parse_field_prefix(c, ctx.p_ver)?;

    let array_dim = c.read_i32::<LittleEndian>()?;
    let property_flags = c.read_u64::<LittleEndian>()?;
    let (category, array_size_enum) = if !ctx.strip_editor_only {
        (Some(read_fname(c)?), Some(c.read_i32::<LittleEndian>()?))
    } else {
        (None, None)
    };
    let rep_offset = if property_flags & CPF_NET != 0 {
        Some(c.read_u16::<LittleEndian>()?)
    } else {
        None
    };

    Ok(PropertyCommon {
        next,
        super_field: pre_756_super,
        array_dim,
        property_flags,
        category,
        array_size_enum,
        rep_offset,
    })
}

fn parse_byte_property(c: &mut Cursor<&Vec<u8>>, ctx: SchemaParseCtx) -> Result<PropertyKind> {
    let common = parse_property_common(c, ctx)?;
    let enum_obj = c.read_i32::<LittleEndian>()?;
    Ok(PropertyKind::Byte { common, enum_obj })
}

fn parse_int_property(c: &mut Cursor<&Vec<u8>>, ctx: SchemaParseCtx) -> Result<PropertyKind> {
    Ok(PropertyKind::Int {
        common: parse_property_common(c, ctx)?,
    })
}

fn parse_bool_property(c: &mut Cursor<&Vec<u8>>, ctx: SchemaParseCtx) -> Result<PropertyKind> {
    Ok(PropertyKind::Bool {
        common: parse_property_common(c, ctx)?,
    })
}

fn parse_float_property(c: &mut Cursor<&Vec<u8>>, ctx: SchemaParseCtx) -> Result<PropertyKind> {
    Ok(PropertyKind::Float {
        common: parse_property_common(c, ctx)?,
    })
}

fn parse_object_property(c: &mut Cursor<&Vec<u8>>, ctx: SchemaParseCtx) -> Result<PropertyKind> {
    let common = parse_property_common(c, ctx)?;
    let property_class = c.read_i32::<LittleEndian>()?;
    Ok(PropertyKind::Object {
        common,
        property_class,
    })
}

fn parse_class_property(c: &mut Cursor<&Vec<u8>>, ctx: SchemaParseCtx) -> Result<PropertyKind> {
    let common = parse_property_common(c, ctx)?;
    let property_class = c.read_i32::<LittleEndian>()?;
    let meta_class = c.read_i32::<LittleEndian>()?;
    Ok(PropertyKind::Class {
        common,
        property_class,
        meta_class,
    })
}

fn parse_component_property(c: &mut Cursor<&Vec<u8>>, ctx: SchemaParseCtx) -> Result<PropertyKind> {
    let common = parse_property_common(c, ctx)?;
    let property_class = c.read_i32::<LittleEndian>()?;
    Ok(PropertyKind::Component {
        common,
        property_class,
    })
}

fn parse_interface_property(c: &mut Cursor<&Vec<u8>>, ctx: SchemaParseCtx) -> Result<PropertyKind> {
    let common = parse_property_common(c, ctx)?;
    let interface_class = c.read_i32::<LittleEndian>()?;
    Ok(PropertyKind::Interface {
        common,
        interface_class,
    })
}

fn parse_name_property(c: &mut Cursor<&Vec<u8>>, ctx: SchemaParseCtx) -> Result<PropertyKind> {
    Ok(PropertyKind::Name {
        common: parse_property_common(c, ctx)?,
    })
}

fn parse_str_property(c: &mut Cursor<&Vec<u8>>, ctx: SchemaParseCtx) -> Result<PropertyKind> {
    Ok(PropertyKind::Str {
        common: parse_property_common(c, ctx)?,
    })
}

fn parse_delegate_property(c: &mut Cursor<&Vec<u8>>, ctx: SchemaParseCtx) -> Result<PropertyKind> {
    let common = parse_property_common(c, ctx)?;
    let function = c.read_i32::<LittleEndian>()?;
    let source_delegate = read_fname(c)?;
    Ok(PropertyKind::Delegate {
        common,
        function,
        source_delegate,
    })
}

fn parse_array_property(c: &mut Cursor<&Vec<u8>>, ctx: SchemaParseCtx) -> Result<PropertyKind> {
    let common = parse_property_common(c, ctx)?;
    let inner = c.read_i32::<LittleEndian>()?;
    Ok(PropertyKind::Array { common, inner })
}

fn parse_map_property(c: &mut Cursor<&Vec<u8>>, ctx: SchemaParseCtx) -> Result<PropertyKind> {
    let common = parse_property_common(c, ctx)?;
    let key = c.read_i32::<LittleEndian>()?;
    let value = c.read_i32::<LittleEndian>()?;
    Ok(PropertyKind::Map { common, key, value })
}

fn parse_struct_property(c: &mut Cursor<&Vec<u8>>, ctx: SchemaParseCtx) -> Result<PropertyKind> {
    let common = parse_property_common(c, ctx)?;
    let struct_obj = c.read_i32::<LittleEndian>()?;
    Ok(PropertyKind::Struct { common, struct_obj })
}

fn read_fname(c: &mut Cursor<&Vec<u8>>) -> Result<FName> {
    Ok(FName {
        name_index: c.read_i32::<LittleEndian>()?,
        name_instance: c.read_i32::<LittleEndian>()?,
    })
}

fn read_fname_array(c: &mut Cursor<&Vec<u8>>) -> Result<Vec<FName>> {
    let n = c.read_i32::<LittleEndian>()?;
    if !(0..=0x10_0000).contains(&n) {
        return Err(Error::new(
            ErrorKind::InvalidData,
            format!("FName TArray: implausible count {}", n),
        ));
    }
    let mut v = Vec::with_capacity(n as usize);
    for _ in 0..n {
        v.push(read_fname(c)?);
    }
    Ok(v)
}

fn read_fname_to_object_map(c: &mut Cursor<&Vec<u8>>) -> Result<Vec<(FName, i32)>> {
    let n = c.read_i32::<LittleEndian>()?;
    if !(0..=0x10_0000).contains(&n) {
        return Err(Error::new(
            ErrorKind::InvalidData,
            format!("TMap<FName,Object*>: implausible count {}", n),
        ));
    }
    let mut v = Vec::with_capacity(n as usize);
    for _ in 0..n {
        let k = read_fname(c)?;
        let val = c.read_i32::<LittleEndian>()?;
        v.push((k, val));
    }
    Ok(v)
}

fn read_object_to_object_map(c: &mut Cursor<&Vec<u8>>) -> Result<Vec<(i32, i32)>> {
    let n = c.read_i32::<LittleEndian>()?;
    if !(0..=0x10_0000).contains(&n) {
        return Err(Error::new(
            ErrorKind::InvalidData,
            format!("TMap<Object*,Object*>: implausible count {}", n),
        ));
    }
    let mut v = Vec::with_capacity(n as usize);
    for _ in 0..n {
        let k = c.read_i32::<LittleEndian>()?;
        let val = c.read_i32::<LittleEndian>()?;
        v.push((k, val));
    }
    Ok(v)
}

pub fn collect_children(entry: &SchemaEntry, pak: &UPKPak) -> Option<Vec<i32>> {
    let head = match entry {
        SchemaEntry::Class { header, .. } => header.children,
        SchemaEntry::State { header, .. } => header.children,
        SchemaEntry::ScriptStruct { header, .. } => header.children,
        SchemaEntry::Struct { header, .. } => header.children,
        SchemaEntry::Function { header, .. } => header.children,
        _ => return None,
    };

    let mut out = Vec::new();
    let cur = head;
    let mut guard = 0;
    while cur != 0 {
        guard += 1;
        if guard > 4096 {
            break;
        }
        out.push(cur);
        let _ = pak;
        break;
    }
    Some(out)
}
