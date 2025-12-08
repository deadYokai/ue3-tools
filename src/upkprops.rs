use std::{collections::HashMap, io::{Cursor, Read, Result, Seek, SeekFrom}};

use byteorder::{LittleEndian, ReadBytesExt};

use crate::upkreader::{read_string, UPKPak};

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub enum PropertyValue {
    Int(i32),
    Bool(bool),
    Float(f32),
    Object(i32),
    Name(String),
    String(String),
    Array(Vec<PropertyValue>),
    Struct(HashMap<String, PropertyValue>),
    Raw(Vec<u8>)
}

#[derive(Debug)]
pub struct Property {
    pub name: String,
    pub prop_type: String,
    pub size: i32,
    pub array_index: i32,
    pub value: PropertyValue
}

pub fn parse_array(reader: &mut Cursor<&Vec<u8>>, pak: &UPKPak) -> Result<PropertyValue> {
    let count = reader.read_i32::<LittleEndian>()?;

    println!("  Array count: {}", count);

    if count <= 0 {
        return Ok(PropertyValue::Array(Vec::new()));
    }

    if count > 10000 {
        println!("  Warning! Sus large array!");
        return Ok(PropertyValue::Array(Vec::new()));
    }

    let start_pos = reader.position();
    let type_index = reader.read_i64::<LittleEndian>()?;

    let has_type_info = type_index > 0
                        && type_index < pak.name_table.len() as i64
                        && pak.name_table[type_index as usize].contains("Property");

    if has_type_info {
        println!("    Array inner type: {}", pak.name_table[type_index as usize].clone());
    } else {
        reader.seek(SeekFrom::Start(start_pos))?;
    }

    let mut elements = Vec::with_capacity(count as usize);
    for _ in 0..count {
        elements.push(PropertyValue::Int(reader.read_i32::<LittleEndian>()?));
    }

    Ok(PropertyValue::Array(elements))
}

pub fn parse_struct(
    reader: &mut Cursor<&Vec<u8>>,
    pak: &UPKPak,
    size: i32
) -> Result<PropertyValue> {
    let struct_name_index = reader.read_i64::<LittleEndian>()?;
    let struct_name = pak.name_table[struct_name_index as usize].clone();

    println!("    Struct type: {}", struct_name);

    let start_pos = reader.position();

    match struct_name.as_str() {
        // Todo DisConv structs
        _ => {
            let mut properties = HashMap::new();

            while reader.position() - start_pos < size as u64 {
                if let Some(prop) = parse_property(reader, pak)? {
                    properties.insert(prop.name.clone(), prop.value);
                } else {
                    break;
                }
            }

            Ok(PropertyValue::Struct(properties))
        }
    }
}

pub fn parse_property(reader: &mut Cursor<&Vec<u8>>, pak: &UPKPak) -> Result<Option<Property>>{
    let mut init_pos = reader.position();
    let mut name_index = reader.read_i64::<LittleEndian>()?;

    if name_index > pak.name_table.len() as i64 && init_pos == 0 {
        init_pos += 4;
        reader.seek(SeekFrom::Start(init_pos))?;
        name_index = reader.read_i64::<LittleEndian>()?;
    }

    if name_index == 0 || name_index > pak.name_table.len() as i64 {
        return Ok(None);
    }

    let prop_name = pak.name_table[name_index as usize].clone();

    if prop_name == "None" {
        return Ok(None);
    }

    if prop_name == "RawData" {
        unimplemented!("Researching how implement");
    }

    let type_index = reader.read_i64::<LittleEndian>()?;
    let prop_type = pak.name_table[type_index as usize].clone();

    let size = reader.read_i32::<LittleEndian>()?;
    let array_index = reader.read_i32::<LittleEndian>()?;

    let value = match prop_type.as_str() {
        "IntProperty" => PropertyValue::Int(reader.read_i32::<LittleEndian>()?),
        "FloatProperty" => PropertyValue::Float(reader.read_f32::<LittleEndian>()?),
        "BoolProperty" => PropertyValue::Bool(reader.read_u8()? != 0),
        "NameProperty" => {
            let idx = reader.read_i64::<LittleEndian>()?;
            PropertyValue::Name(pak.name_table[idx as usize].clone())
        },
        "StrProperty" => PropertyValue::String(read_string(reader)?),
        "ObjectProperty" => PropertyValue::Object(reader.read_i32::<LittleEndian>()?),
        "ArrayProperty" => parse_array(reader, pak)?,
        "StructProperty" => parse_struct(reader, pak, size)?,
        _ => {
            let mut buf = vec![0u8; size as usize];
            reader.read_exact(&mut buf)?;
            PropertyValue::Raw(buf)
        }
    };

    Ok(Some(Property {
        name: prop_name,
        prop_type,
        size,
        array_index,
        value
    }))

}

