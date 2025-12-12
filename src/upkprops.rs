use std::{collections::HashMap, io::{Cursor, Read, Result, Seek, SeekFrom}};

use byteorder::{ByteOrder, LittleEndian, ReadBytesExt};

use crate::upkreader::{read_string, UPKPak};

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub enum PropertyValue {
    Byte(u8),
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
}

#[derive(Debug)]
pub struct Property {
    pub name: String,
    pub prop_type: String,
    pub size: i32,
    pub array_index: i32,
    pub value: PropertyValue,
    pub enum_name: Option<String>
}

pub fn parse_array(reader: &mut Cursor<&Vec<u8>>, pak: &UPKPak, size: i32) -> Result<PropertyValue> {
    let start_pos = reader.position();
    let count = reader.read_i32::<LittleEndian>()?;

    println!("  Array count: {}", count);

    if count < 0 {
        println!("  ERR: invalid array count: {}", count);
        return Ok(PropertyValue::Array(Vec::new()));
    }

    if count == 0 { 
        return Ok(PropertyValue::Array(Vec::new()));
    }

    if count > 1_000_000 {
        println!("  Warning! Sus large array!");
        return Ok(PropertyValue::Array(Vec::new()));
    }

    let bytes_read = reader.position() - start_pos;
    let remaining_bytes = (size as u64).saturating_sub(bytes_read);

    if remaining_bytes == 0 {
        println!("  Warning: No data in array elements");
        return Ok(PropertyValue::Array(Vec::new()));
    }

    let bytes_per_element = remaining_bytes / count as u64;

    let mut elements = Vec::with_capacity(count as usize);

    match bytes_per_element {
        1 => {
            for _ in 0..count {
                let val = reader.read_u8()?;
                elements.push(PropertyValue::Byte(val));
            }
        }
        4 => {
            let pos = reader.position();
            let first_val = reader.read_i32::<LittleEndian>()?;
            reader.seek(SeekFrom::Start(pos))?;

            let is_obj = first_val < 0 || (first_val > 0 && first_val < 10000);

            if is_obj {
                for _ in 0..count {
                    let obj_ref = reader.read_i32::<LittleEndian>()?;
                    elements.push(PropertyValue::Object(obj_ref));
                }
            } else {
                for _ in 0..count {
                    let val = reader.read_i32::<LittleEndian>()?;
                    elements.push(PropertyValue::Int(val));
                }
            }
        }
        8 => {
            for _ in 0..count {
                let idx = reader.read_i64::<LittleEndian>()?;
                if idx >= 0 && idx < pak.name_table.len() as i64 {
                    let name = pak.name_table[idx as usize].clone();
                    elements.push(PropertyValue::Name(name));
                } else {
                    elements.push(PropertyValue::Int(idx as i32));
                }
            }
        }
        _ => {
            let target_end = start_pos + size as u64;
            let mut element_count = 0;

            while reader.position() < target_end && element_count < count {
                let element_start = reader.position();
                let remaining = target_end - element_start;
                let left = count - element_count;

                if left > 0 {
                    let est_size = remaining / left as u64;
                    
                    if est_size > 0 && est_size < 1_000_000 {
                        let mut el_data = vec![0u8; est_size as usize];
                        reader.read_exact(&mut el_data)?;
                        elements.push(PropertyValue::Raw(el_data));
                        element_count += 1;
                    } else {
                        break;
                    }
                } else {
                    break;
                }
            }
        }
    }

    let bytes_consumed = reader.position();
    if bytes_consumed != size as u64 {
        println!("  Warning: size mismatch - expected {}, consumed {}", size, bytes_consumed);
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

    let type_index = reader.read_i64::<LittleEndian>()?;
    let prop_type = pak.name_table[type_index as usize].clone();

    let size = reader.read_i32::<LittleEndian>()?;
    let array_index = reader.read_i32::<LittleEndian>()?;

    let enum_name = if prop_type == "ByteProperty" {
        let enum_index = reader.read_i64::<LittleEndian>()?;
        if enum_index > 0 && enum_index < pak.name_table.len() as i64 {
            let name = pak.name_table[enum_index as usize].clone();
            Some(name)
        } else {
            None
        }
    } else {
        None
    };

    let value = match prop_type.as_str() {
        "IntProperty" => PropertyValue::Int(reader.read_i32::<LittleEndian>()?),
        "FloatProperty" => PropertyValue::Float(reader.read_f32::<LittleEndian>()?),
        "BoolProperty" => PropertyValue::Bool(reader.read_u8()? != 0),
        "ByteProperty" => {
            // Size
            // 1 - just a simple byte
            // 8 - enum
            if enum_name.is_some() {
                let enum_val_idx = reader.read_i64::<LittleEndian>()?;
                if enum_val_idx >= 0 && enum_val_idx < pak.name_table.len() as i64 {
                    let enum_val_name = pak.name_table[enum_val_idx as usize].clone();
                    PropertyValue::Name(enum_val_name)
                } else {
                    PropertyValue::Int(enum_val_idx as i32)
                }
            } else {
                let val = reader.read_u8()?;
                PropertyValue::Byte(val)
            }
        },
        "NameProperty" => {
            let idx = reader.read_i64::<LittleEndian>()?;
            PropertyValue::Name(pak.name_table[idx as usize].clone())
        },
        "StrProperty" => PropertyValue::String(read_string(reader)?),
        "ObjectProperty" => PropertyValue::Object(reader.read_i32::<LittleEndian>()?),
        "ArrayProperty" => parse_array(reader, pak, size)?,
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
        value,
        enum_name
    }))

}

