use std::{collections::HashMap, io::{Cursor, Read, Result, Seek, SeekFrom, Write}};

use byteorder::{ByteOrder, LittleEndian, ReadBytesExt};
use serde::{Deserialize, Serialize};

use crate::upkreader::{read_string, FName, UPKPak};

pub const VER_BYTEPROP_SERIALIZE_ENUM: i16 = 633;
pub const VER_PROPERTYTAG_BOOL_OPTIMIZATION: i16 = 673;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[allow(dead_code)]
pub enum PropertyValue {
    None,
    Byte(u8),
    Int(i32),
    Bool(bool),
    Float(f32),
    Object(i32),
    Name(FName),
    String(String),
    Array(Vec<PropertyValue>),
    Struct(Vec<(String, PropertyValue)>),
    Raw(Vec<u8>),
}

trait IntoArrayOrRaw {
    fn into_array_or_raw(self) -> PropertyValue;
}

impl IntoArrayOrRaw for PropertyValue {
    fn into_array_or_raw(self) -> PropertyValue { self }
}

impl PropertyValue {
    pub fn as_vec(&self) -> Option<&Vec<PropertyValue>> {
        if let PropertyValue::Array(a) = self {
            Some(a)
        } else {
            None
        }
    }

    pub fn as_byte(&self) -> Option<u8> {
        if let PropertyValue::Byte(b) = self {
            Some(*b)
        } else {
            None
        }
    }

    pub fn write_all<W: Write>(&self, writer: &mut W) -> Result<()> {
        match self {
            PropertyValue::None => unreachable!(),
            PropertyValue::Byte(b) => writer.write_all(&[*b])?,
            PropertyValue::Int(i) => writer.write_all(&i.to_le_bytes())?,
            PropertyValue::Bool(b) => writer.write_all(&[if *b {1u8} else {0u8}])?,
            PropertyValue::Float(f) => writer.write_all(&f.to_le_bytes())?,
            PropertyValue::Object(id) => writer.write_all(&id.to_le_bytes())?,
            PropertyValue::Raw(data) => writer.write_all(data)?,
            PropertyValue::Name(fname) => {
                writer.write_all(&fname.name_index.to_le_bytes())?;
                writer.write_all(&fname.name_instance.to_le_bytes())?;
            },
            PropertyValue::String(_) => {
                todo!();
            },
            PropertyValue::Array(arr) => {
                writer.write_all(&(arr.len() as i32).to_le_bytes())?;
                for el in arr { el.write_all(writer)?; }
            },
            PropertyValue::Struct(fields) => {
                for (_, v) in fields { v.write_all(writer)?; }
            }
        }

        Ok(())
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let mut buf = Vec::new();
        self.write_all(&mut buf).expect("");
        buf
    }

}

#[derive(Debug, Serialize, Deserialize)]
pub struct Property {
    pub name: String,
    pub prop_type: String,
    pub size: i32,
    pub array_index: i32,
    pub value: PropertyValue,
    pub enum_name: Option<String>,
    pub struct_name: Option<String>,
}

impl Property {
    pub fn to_bytes(&self) -> Vec<u8> {
        todo!()
    }
}

fn read_fname(reader: &mut Cursor<&Vec<u8>>) -> Result<FName> {
    Ok(FName {
        name_index: reader.read_i32::<LittleEndian>()?,
        name_instance: reader.read_i32::<LittleEndian>()?
    })
}

fn resolve_fname(fname: &FName, pak: &UPKPak) -> Option<String> {
    let name = pak.name_table.get(fname.name_index as usize)?;
    if fname.name_instance > 0 {
        Some(format!("{}_{}", name, fname.name_instance - 1))
    } else {
        Some(name.clone())
    }
}

pub fn parse_array(
    reader: &mut Cursor<&Vec<u8>>,
    pak: &UPKPak,
    size: i32,
    ver: i16
) -> Result<PropertyValue> {    
    let start_pos = reader.position();
    let count = reader.read_i32::<LittleEndian>()?;

    if count < 0 {
        eprintln!("Invalid array count: {}", count);
        return Ok(PropertyValue::Array(Vec::new()));
    }

    if count == 0 { 
        return Ok(PropertyValue::Array(Vec::new()));
    }

    if count > 1_000_000 {
        eprintln!("Warning: Very large array count: {}", count);
        let skip = (size as i64) - 4;
        if skip > 0 { reader.seek(SeekFrom::Current(skip))?; }
        return Ok(PropertyValue::Array(vec![]));
    }

    let data_bytes = (size as u64).saturating_sub(4);
    if data_bytes == 0 {
        return Ok(PropertyValue::Array(vec![]));
    }

    if data_bytes % count as u64 != 0 {
        eprintln!("warn: array data_bytes={data_bytes} not divisible by count={count}");
        let mut buf = vec![0u8; data_bytes as usize];
        reader.read_exact(&mut buf)?;
        return Ok(PropertyValue::Raw(buf).into_array_or_raw());
    }

    let elem_size = data_bytes / count as u64;
    let mut elements = Vec::with_capacity(count as usize);

    match elem_size {
        1 => {
            for _ in 0..count {
                let val = reader.read_u8()?;
                elements.push(PropertyValue::Byte(val));
            }
        }
        4 => {
            let pos0 = reader.position();
            let raw = reader.read_i32::<LittleEndian>()?;
            reader.seek(SeekFrom::Start(pos0))?;

            let is_obj = raw < 0 || (raw > 0 && raw < 65536);
            let as_f   = f32::from_bits(raw as u32);
            let is_flt = !is_obj && as_f.is_finite() && as_f.abs() < 1e10;
            for _ in 0..count {
                elements.push(if is_obj {
                    PropertyValue::Object(reader.read_i32::<LittleEndian>()?)
                } else if is_flt {
                    PropertyValue::Float(reader.read_f32::<LittleEndian>()?)
                } else {
                    PropertyValue::Int(reader.read_i32::<LittleEndian>()?)
                });
            }
        }
        8 => {
            for _ in 0..count {
                let fname = read_fname(reader)?;
                elements.push(PropertyValue::Name(fname));
            }
        }
        12 => {
            let pos0 = reader.position();
            let raw  = reader.read_u32::<LittleEndian>()?;
            reader.seek(SeekFrom::Start(pos0))?;
            let is_flt = f32::from_bits(raw).is_finite();

            for _ in 0..count {
                let (x_raw, y_raw, z_raw) = (
                    reader.read_u32::<LittleEndian>()?,
                    reader.read_u32::<LittleEndian>()?,
                    reader.read_u32::<LittleEndian>()?,
                );
                elements.push(PropertyValue::Struct(if is_flt {
                    vec![
                        ("X".into(), PropertyValue::Float(f32::from_bits(x_raw))),
                        ("Y".into(), PropertyValue::Float(f32::from_bits(y_raw))),
                        ("Z".into(), PropertyValue::Float(f32::from_bits(z_raw))),
                    ]
                } else {
                    vec![
                        ("Pitch".into(), PropertyValue::Int(x_raw as i32)),
                        ("Yaw".into(),   PropertyValue::Int(y_raw as i32)),
                        ("Roll".into(),  PropertyValue::Int(z_raw as i32)),
                    ]
                }));
            }
        }
        16 => {
            for _ in 0..count {
                let a = reader.read_u32::<LittleEndian>()?;
                let b = reader.read_u32::<LittleEndian>()?;
                let c = reader.read_u32::<LittleEndian>()?;
                let d = reader.read_u32::<LittleEndian>()?;
                let as_f = f32::from_bits(a);
                let fields = if as_f.is_finite() {
                    vec![
                        ("X".into(), PropertyValue::Float(f32::from_bits(a))),
                        ("Y".into(), PropertyValue::Float(f32::from_bits(b))),
                        ("Z".into(), PropertyValue::Float(f32::from_bits(c))),
                        ("W".into(), PropertyValue::Float(f32::from_bits(d))),
                    ]
                } else {
                    vec![
                        ("A".into(), PropertyValue::Int(a as i32)),
                        ("B".into(), PropertyValue::Int(b as i32)),
                        ("C".into(), PropertyValue::Int(c as i32)),
                        ("D".into(), PropertyValue::Int(d as i32)),
                    ]
                };
                elements.push(PropertyValue::Struct(fields));
            }
        }
        _ => {
            let start_pos = reader.position();
            let total_end = start_pos + data_bytes;

            let mut parsed_as_structs = false;

            {
                let probe_start = reader.position();
                let probe_end   = probe_start + elem_size;
                let mut ok = true;
                let mut probe_fields: Vec<(String, PropertyValue)> = Vec::new();

                loop {
                    if reader.position() >= probe_end { break; }
                    match parse_property(reader, pak, ver) {
                        Ok(Some(p)) if p.name == "None" => break,
                        Ok(Some(p)) => probe_fields.push((p.name.clone(), p.value)),
                        _ => { ok = false; break; }
                    }
                }

                if ok {
                    reader.seek(SeekFrom::Start(probe_start))?;
                    parsed_as_structs = true;
                } else {
                    reader.seek(SeekFrom::Start(probe_start))?;
                }
            }

            if parsed_as_structs {
                for _ in 0..count {
                    let elem_end = reader.position() + elem_size;
                    let mut fields = Vec::new();
                    loop {
                        if reader.position() >= elem_end { break; }
                        match parse_property(reader, pak, ver)? {
                            Some(p) if p.name == "None" => break,
                            Some(p) => fields.push((p.name.clone(), p.value)),
                            None    => break,
                        }
                    }
                    if reader.position() < elem_end {
                        reader.seek(SeekFrom::Start(elem_end))?;
                    }
                    elements.push(PropertyValue::Struct(fields));
                }
            } else {
                for _ in 0..count {
                    let mut buf = vec![0u8; elem_size as usize];
                    reader.read_exact(&mut buf)?;
                    elements.push(PropertyValue::Raw(buf));
                }
            }

        }
    }

    Ok(PropertyValue::Array(elements))
}

pub fn parse_struct(
    r: &mut Cursor<&Vec<u8>>,
    pak: &UPKPak,
    size: i32,
    struct_name: &str,
    ver: i16
) -> Result<PropertyValue> {
    match struct_name {
        "Guid" => {
            let mut f = Vec::with_capacity(4);
            for lbl in &["A","B","C","D"] {
                let v = r.read_u32::<LittleEndian>()?;
                f.push((lbl.to_string(), PropertyValue::Int(v as i32)));
            }
            Ok(PropertyValue::Struct(f))
        }
        "Vector" => {
            let x = r.read_f32::<LittleEndian>()?;
            let y = r.read_f32::<LittleEndian>()?;
            let z = r.read_f32::<LittleEndian>()?;
            Ok(PropertyValue::Struct(vec![
                ("X".into(), PropertyValue::Float(x)),
                ("Y".into(), PropertyValue::Float(y)),
                ("Z".into(), PropertyValue::Float(z)),
            ]))
        }
        "Vector2D" => {
            let x = r.read_f32::<LittleEndian>()?;
            let y = r.read_f32::<LittleEndian>()?;
            Ok(PropertyValue::Struct(vec![
                ("X".into(), PropertyValue::Float(x)),
                ("Y".into(), PropertyValue::Float(y)),
            ]))
        }
        "Vector4" | "Quat" => {
            let mut f = Vec::with_capacity(4);
            for lbl in &["X","Y","Z","W"] {
                f.push((lbl.to_string(), PropertyValue::Float(r.read_f32::<LittleEndian>()?)));
            }
            Ok(PropertyValue::Struct(f))
        }
        "Rotator" => {
            Ok(PropertyValue::Struct(vec![
                ("Pitch".into(), PropertyValue::Int(r.read_i32::<LittleEndian>()?)),
                ("Yaw".into(),   PropertyValue::Int(r.read_i32::<LittleEndian>()?)),
                ("Roll".into(),  PropertyValue::Int(r.read_i32::<LittleEndian>()?)),
            ]))
        }
        "Color" => {
            Ok(PropertyValue::Struct(vec![
                ("B".into(), PropertyValue::Byte(r.read_u8()?)),
                ("G".into(), PropertyValue::Byte(r.read_u8()?)),
                ("R".into(), PropertyValue::Byte(r.read_u8()?)),
                ("A".into(), PropertyValue::Byte(r.read_u8()?)),
            ]))
        }
        "LinearColor" => {
            let mut f = Vec::with_capacity(4);
            for lbl in &["R","G","B","A"] {
                f.push((lbl.to_string(), PropertyValue::Float(r.read_f32::<LittleEndian>()?)));
            }
            Ok(PropertyValue::Struct(f))
        }
        "Box" => {
            // Min (Vector) + Max (Vector) + IsValid (byte)
            let min_x = r.read_f32::<LittleEndian>()?;
            let min_y = r.read_f32::<LittleEndian>()?;
            let min_z = r.read_f32::<LittleEndian>()?;
            let max_x = r.read_f32::<LittleEndian>()?;
            let max_y = r.read_f32::<LittleEndian>()?;
            let max_z = r.read_f32::<LittleEndian>()?;
            let valid = r.read_u8()?;
            Ok(PropertyValue::Struct(vec![
                ("Min".into(), PropertyValue::Struct(vec![
                    ("X".into(), PropertyValue::Float(min_x)),
                    ("Y".into(), PropertyValue::Float(min_y)),
                    ("Z".into(), PropertyValue::Float(min_z)),
                ])),
                ("Max".into(), PropertyValue::Struct(vec![
                    ("X".into(), PropertyValue::Float(max_x)),
                    ("Y".into(), PropertyValue::Float(max_y)),
                    ("Z".into(), PropertyValue::Float(max_z)),
                ])),
                ("IsValid".into(), PropertyValue::Byte(valid)),
            ]))
        }
        _ => {
            let start   = r.position();
            let end     = start + size as u64;
            let mut fields = Vec::new();

            while r.position() < end {
                match parse_property(r, pak, ver)? {
                    None => break,
                    Some(prop) if prop.name == "None" => break,
                    Some(prop) => fields.push((prop.name.clone(), prop.value)),
                }
            }

            let consumed = r.position().saturating_sub(start);
            if consumed < size as u64 {
                r.seek(SeekFrom::Current((size as u64 - consumed) as i64))?;
            }

            Ok(PropertyValue::Struct(fields))
        }
    }
}

pub fn parse_property(
    r: &mut Cursor<&Vec<u8>>,
    pak: &UPKPak,
    ver: i16
) -> Result<Option<Property>>{
    let name_pos = r.position();

    {
        let end = r.seek(SeekFrom::End(0))?;
        r.seek(SeekFrom::Start(name_pos))?;
        if name_pos + 8 > end {
            return Ok(None);
        }
    }

    let prop_fname = read_fname(r)?;
    
    if prop_fname.name_index < 0
        || prop_fname.name_index as usize >= pak.name_table.len()
    {
        return Ok(None);
    }

    let prop_name = match resolve_fname(&prop_fname, pak) {
        Some(n) => n,
        None => return Ok(None),
    };

    if prop_name == "None" {
        return Ok(Some(Property {
            name: "None".into(), prop_type: "None".into(),
            size: 0, array_index: 0,
            value: PropertyValue::None,
            enum_name: None, struct_name: None,
        }));
    }


    let type_fname = read_fname(r)?;

    if type_fname.name_index < 0
        || type_fname.name_index as usize >= pak.name_table.len()
    {
        return Ok(None);
    }

    let prop_type = match resolve_fname(&type_fname, pak) {
        Some(t) => t,
        None    => return Ok(None),
    };

    const KNOWN_TYPES: &[&str] = &[
        "IntProperty", "FloatProperty", "BoolProperty", "ByteProperty",
        "NameProperty", "StrProperty",  "ObjectProperty", "ComponentProperty",
        "InterfaceProperty", "ClassProperty", "ArrayProperty", "StructProperty",
        "DelegateProperty", "MapProperty",
    ];

    if !KNOWN_TYPES.contains(&prop_type.as_str()) {
        return Ok(None);
    }

    let size        = r.read_i32::<LittleEndian>()?;
    let array_index = r.read_i32::<LittleEndian>()?;

    let mut struct_name: Option<String> = None;
    let mut bool_val:    Option<bool>   = None;
    let mut enum_name:   Option<String> = None;
    
    match prop_type.as_str() {
        "StructProperty" => {
            let sn = read_fname(r)?;
            struct_name = resolve_fname(&sn, pak);
        }
        "BoolProperty" => {
            if ver >= VER_PROPERTYTAG_BOOL_OPTIMIZATION {
                bool_val = Some(r.read_u8()? != 0);
            } else {
                bool_val = Some(r.read_u32::<LittleEndian>()? != 0);
            }
        }
        "ByteProperty" if ver >= VER_BYTEPROP_SERIALIZE_ENUM => {
            let en = read_fname(r)?;
            enum_name = resolve_fname(&en, pak);
            if enum_name.as_deref() == Some("None") {
                enum_name = None;
            }
        }
        _ => {}
    }

    // ── Value ────────────────────────────────────────────────────────────────
    let value_start = r.position();

    let value = match prop_type.as_str() {
        "IntProperty"   => PropertyValue::Int(r.read_i32::<LittleEndian>()?),
        "FloatProperty" => PropertyValue::Float(r.read_f32::<LittleEndian>()?),

        "BoolProperty" => PropertyValue::Bool(bool_val.unwrap_or(false)),

        "ByteProperty" => {
            if let Some(ref en) = enum_name {
                let _ = en;
                PropertyValue::Name(read_fname(r)?)
            } else {
                PropertyValue::Byte(r.read_u8()?)
            }
        }

        "NameProperty"   => PropertyValue::Name(read_fname(r)?),
        "StrProperty"    => PropertyValue::String(read_string(r)?),
        "ObjectProperty" | "ComponentProperty" | "InterfaceProperty" => {
            PropertyValue::Object(r.read_i32::<LittleEndian>()?)
        }
        "ClassProperty"  => PropertyValue::Object(r.read_i32::<LittleEndian>()?),

        "ArrayProperty"  => parse_array(r, pak, size, ver)?,

        "StructProperty" => {
            let sn = struct_name.as_deref().unwrap_or("Unknown");
            parse_struct(r, pak, size, sn, ver)?
        }

        "DelegateProperty" => {
            // object ref (i32) + function FName (8 bytes) = 12 bytes
            let obj  = r.read_i32::<LittleEndian>()?;
            let func = read_fname(r)?;
            PropertyValue::Struct(vec![
                ("Object".into(), PropertyValue::Object(obj)),
                ("Function".into(), PropertyValue::Name(func)),
            ])
        }

        "MapProperty" => {
            // Not common; just swallow the bytes
            let mut buf = vec![0u8; size as usize];
            r.read_exact(&mut buf)?;
            PropertyValue::Raw(buf)
        }

        _ => {
            eprintln!("warn: unknown property type '{prop_type}' for '{prop_name}' (size={size})");
            if size > 0 && size < 65536 {
                let mut buf = vec![0u8; size as usize];
                r.read_exact(&mut buf)?;
                PropertyValue::Raw(buf)
            } else {
                return Ok(None);
            }
        }
    };

    if prop_type != "BoolProperty" {
        let consumed = (r.position() - value_start) as i32;
        if consumed != size && !matches!(prop_type.as_str(),
            "ArrayProperty" | "StructProperty" | "StrProperty" | "DelegateProperty" | "MapProperty")
        {
            eprintln!(
                "warn: '{prop_name}' ({prop_type}): read {consumed} bytes but tag says {size}"
            );
            r.seek(SeekFrom::Start(value_start + size as u64))?;
        }
    }

    Ok(Some(Property {
        name: prop_name, prop_type,
        size, array_index, value,
        enum_name, struct_name 
    }))
}

