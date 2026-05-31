use std::io::{Cursor, Error, ErrorKind, Read, Result, Seek, SeekFrom, Write};

use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};
use serde::{Deserialize, Serialize};

use crate::{
    schema::{PropertyKind, SchemaEntry},
    schemadb::{ResolvedRef, SchemaDb},
    upkreader::{FName, UPKPak, read_string, write_fstring},
    versions::{
        VER_BYTEPROP_SERIALIZE_ENUM as V_BYTE_ENUM, VER_PROPERTYTAG_BOOL_OPTIMIZATION as V_BOOL_OPT,
    },
};

fn is_builtin_atomic(name: &str) -> bool {
    matches!(
        name,
        "Vector"
            | "Vector2D"
            | "Vector4"
            | "Quat"
            | "Rotator"
            | "Color"
            | "LinearColor"
            | "Box"
            | "Box2D"
            | "BoxSphereBounds"
            | "Matrix"
            | "Plane"
            | "Sphere"
            | "Guid"
            | "IntPoint"
            | "TwoVectors"
            | "InterpCurvePointFloat"
            | "InterpCurvePointVector"
            | "InterpCurvePointVector2D"
            | "InterpCurvePointTwoVectors"
            | "InterpCurvePointQuat"
    )
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PropertyValue {
    None,
    Byte(u8),
    Int(i32),
    Bool(bool),
    Float(f32),

    Object(i32),

    ObjectRef(String),
    Name(FName),

    EnumLabel(String),
    String(String),
    Array(Vec<PropertyValue>),

    Struct(Vec<Property>),

    AtomicStruct(Vec<(String, PropertyValue)>),

    Raw(Vec<u8>),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Property {
    pub name: String,
    pub prop_type: String,
    pub size: i32,
    pub array_index: i32,
    pub value: PropertyValue,
    pub enum_name: Option<String>,
    pub struct_name: Option<String>,
}

#[derive(Clone)]
pub struct PropertyCtx<'a> {
    pub pak: &'a UPKPak,
    pub ver: i16,
    pub db: Option<&'a SchemaDb>,
    pub owner: Option<ResolvedRef>,
}

impl<'a> PropertyCtx<'a> {
    pub fn legacy(pak: &'a UPKPak, ver: i16) -> Self {
        Self {
            pak,
            ver,
            db: None,
            owner: None,
        }
    }
    pub fn with_owner(&self, owner: ResolvedRef) -> Self {
        let mut c = self.clone();
        c.owner = Some(owner);
        c
    }
    pub fn drop_owner(&self) -> Self {
        let mut c = self.clone();
        c.owner = None;
        c
    }
}

fn read_fname(r: &mut Cursor<&Vec<u8>>) -> Result<FName> {
    Ok(FName {
        name_index: r.read_i32::<LittleEndian>()?,
        name_instance: r.read_i32::<LittleEndian>()?,
    })
}

fn write_fname<W: Write>(w: &mut W, f: &FName) -> Result<()> {
    w.write_i32::<LittleEndian>(f.name_index)?;
    w.write_i32::<LittleEndian>(f.name_instance)?;
    Ok(())
}

fn resolve_fname(f: &FName, pak: &UPKPak) -> Option<String> {
    let n = pak.name_table.get(f.name_index as usize)?;
    if f.name_instance > 0 {
        Some(format!("{}_{}", n, f.name_instance - 1))
    } else {
        Some(n.clone())
    }
}

fn find_name(pak: &UPKPak, name: &str) -> Result<i32> {
    pak.name_table
        .iter()
        .position(|n| n == name)
        .map(|i| i as i32)
        .ok_or_else(|| {
            Error::new(
                ErrorKind::NotFound,
                format!("name '{name}' not in package name table (Phase 4 will append)"),
            )
        })
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

    pub fn write_all<W: Write + Seek>(&self, w: &mut W, pak: &UPKPak, ver: i16) -> Result<()> {
        use PropertyValue::*;
        match self {
            None => {}
            Byte(b) => w.write_u8(*b)?,
            Int(i) => w.write_i32::<LittleEndian>(*i)?,

            Bool(b) => w.write_u8(if *b { 1 } else { 0 })?,
            Float(f) => w.write_f32::<LittleEndian>(*f)?,
            Object(o) => w.write_i32::<LittleEndian>(*o)?,
            ObjectRef(_) => {
                return Err(Error::new(
                    ErrorKind::InvalidInput,
                    "ObjectRef must be re-resolved to Object before write (Phase 4)",
                ));
            }
            Name(f) => write_fname(w, f)?,
            EnumLabel(label) => {
                let val = label.rsplit("::").next().unwrap_or(label);
                let idx = find_name(pak, val)?;
                w.write_i32::<LittleEndian>(idx)?;
                w.write_i32::<LittleEndian>(0)?;
            }
            String(s) => write_fstring(w, s)?,
            Raw(d) => w.write_all(d)?,
            Array(arr) => {
                w.write_i32::<LittleEndian>(arr.len() as i32)?;
                for el in arr {
                    el.write_all(w, pak, ver)?;
                }
            }
            Struct(fields) => {
                for p in fields {
                    p.write(w, pak, ver)?;
                }

                let none_idx = find_name(pak, "None")?;
                w.write_i32::<LittleEndian>(none_idx)?;
                w.write_i32::<LittleEndian>(0)?;
            }
            AtomicStruct(fields) => {
                for (_, v) in fields {
                    v.write_all(w, pak, ver)?;
                }
            }
        }
        Ok(())
    }
}

impl Property {
    pub fn write<W: Write + Seek>(&self, w: &mut W, pak: &UPKPak, ver: i16) -> Result<()> {
        let name_idx = find_name(pak, &self.name)?;
        w.write_i32::<LittleEndian>(name_idx)?;
        w.write_i32::<LittleEndian>(0)?;
        if self.name == "None" {
            return Ok(());
        }

        let type_idx = find_name(pak, &self.prop_type)?;
        w.write_i32::<LittleEndian>(type_idx)?;
        w.write_i32::<LittleEndian>(0)?;

        let size_offset = w.stream_position()?;
        w.write_i32::<LittleEndian>(0)?;
        w.write_i32::<LittleEndian>(self.array_index)?;

        let mut bool_in_tag = false;
        match self.prop_type.as_str() {
            "StructProperty" => {
                let sn = self.struct_name.as_deref().unwrap_or("None");
                let sn_idx = find_name(pak, sn)?;
                w.write_i32::<LittleEndian>(sn_idx)?;
                w.write_i32::<LittleEndian>(0)?;
            }
            "BoolProperty" => {
                bool_in_tag = true;
                let b = matches!(self.value, PropertyValue::Bool(true));
                if ver >= V_BOOL_OPT {
                    w.write_u8(if b { 1 } else { 0 })?;
                } else {
                    w.write_u32::<LittleEndian>(if b { 1 } else { 0 })?;
                }
            }
            "ByteProperty" if ver >= V_BYTE_ENUM => {
                let en = self.enum_name.as_deref().unwrap_or("None");
                let en_idx = find_name(pak, en)?;
                w.write_i32::<LittleEndian>(en_idx)?;
                w.write_i32::<LittleEndian>(0)?;
            }
            _ => {}
        }

        let value_start = w.stream_position()?;
        if !bool_in_tag {
            self.value.write_all(w, pak, ver)?;
        }
        let value_end = w.stream_position()?;

        let size = if bool_in_tag {
            0
        } else {
            (value_end - value_start) as i32
        };
        w.seek(SeekFrom::Start(size_offset))?;
        w.write_i32::<LittleEndian>(size)?;
        w.seek(SeekFrom::Start(value_end))?;
        Ok(())
    }

    pub fn to_bytes(&self, pak: &UPKPak, ver: i16) -> Result<Vec<u8>> {
        let mut buf = Vec::new();
        let mut c = Cursor::new(&mut buf);
        self.write(&mut c, pak, ver)?;
        Ok(buf)
    }
}

pub fn parse_property(
    r: &mut Cursor<&Vec<u8>>,
    pak: &UPKPak,
    ver: i16,
) -> Result<Option<Property>> {
    parse_property_ctx(r, &PropertyCtx::legacy(pak, ver))
}

pub fn parse_property_ctx(r: &mut Cursor<&Vec<u8>>, ctx: &PropertyCtx) -> Result<Option<Property>> {
    let name_pos = r.position();
    let end = {
        let e = r.seek(SeekFrom::End(0))?;
        r.seek(SeekFrom::Start(name_pos))?;
        e
    };
    if name_pos + 8 > end {
        return Ok(None);
    }

    let prop_fname = read_fname(r)?;
    if prop_fname.name_index < 0 || prop_fname.name_index as usize >= ctx.pak.name_table.len() {
        return Ok(None);
    }
    let prop_name = match resolve_fname(&prop_fname, ctx.pak) {
        Some(n) => n,
        None => return Ok(None),
    };
    if prop_name == "None" {
        return Ok(Some(Property {
            name: "None".into(),
            prop_type: "None".into(),
            size: 0,
            array_index: 0,
            value: PropertyValue::None,
            enum_name: None,
            struct_name: None,
        }));
    }

    let type_fname = read_fname(r)?;
    if type_fname.name_index < 0 || type_fname.name_index as usize >= ctx.pak.name_table.len() {
        return Ok(None);
    }
    let prop_type = match resolve_fname(&type_fname, ctx.pak) {
        Some(t) => t,
        None => return Ok(None),
    };

    const KNOWN: &[&str] = &[
        "IntProperty",
        "FloatProperty",
        "BoolProperty",
        "ByteProperty",
        "NameProperty",
        "StrProperty",
        "ObjectProperty",
        "ComponentProperty",
        "InterfaceProperty",
        "ClassProperty",
        "ArrayProperty",
        "StructProperty",
        "DelegateProperty",
        "MapProperty",
    ];
    if !KNOWN.contains(&prop_type.as_str()) {
        return Ok(None);
    }

    let size = r.read_i32::<LittleEndian>()?;
    let array_index = r.read_i32::<LittleEndian>()?;

    let mut struct_name: Option<String> = None;
    let mut bool_val: Option<bool> = None;
    let mut enum_name: Option<String> = None;
    match prop_type.as_str() {
        "StructProperty" => {
            let sn = read_fname(r)?;
            struct_name = resolve_fname(&sn, ctx.pak);
        }
        "BoolProperty" => {
            if ctx.ver >= V_BOOL_OPT {
                bool_val = Some(r.read_u8()? != 0);
            } else {
                bool_val = Some(r.read_u32::<LittleEndian>()? != 0);
            }
        }
        "ByteProperty" if ctx.ver >= V_BYTE_ENUM => {
            let en = read_fname(r)?;
            enum_name = resolve_fname(&en, ctx.pak);
            if enum_name.as_deref() == Some("None") {
                enum_name = None;
            }
        }
        _ => {}
    }

    let value_start = r.position();
    let value = match prop_type.as_str() {
        "IntProperty" => PropertyValue::Int(r.read_i32::<LittleEndian>()?),
        "FloatProperty" => PropertyValue::Float(r.read_f32::<LittleEndian>()?),
        "BoolProperty" => PropertyValue::Bool(bool_val.unwrap_or(false)),
        "ByteProperty" => {
            if let Some(ref en) = enum_name {
                let fn_ = read_fname(r)?;
                let val_name = resolve_fname(&fn_, ctx.pak).unwrap_or_default();
                PropertyValue::EnumLabel(format!("{en}::{val_name}"))
            } else {
                PropertyValue::Byte(r.read_u8()?)
            }
        }
        "NameProperty" => PropertyValue::Name(read_fname(r)?),
        "StrProperty" => PropertyValue::String(read_string(r)?),
        "ObjectProperty" | "ComponentProperty" | "InterfaceProperty" | "ClassProperty" => {
            PropertyValue::Object(r.read_i32::<LittleEndian>()?)
        }
        "ArrayProperty" => parse_array_ctx(r, ctx, size, &prop_name)?,
        "StructProperty" => {
            let sn = struct_name.as_deref().unwrap_or("Unknown");
            parse_struct_ctx(r, ctx, size, sn, &prop_name)?
        }
        "DelegateProperty" => {
            let obj = r.read_i32::<LittleEndian>()?;
            let func = read_fname(r)?;

            PropertyValue::AtomicStruct(vec![
                ("Object".into(), PropertyValue::Object(obj)),
                ("Function".into(), PropertyValue::Name(func)),
            ])
        }
        "MapProperty" => {
            let mut buf = vec![0u8; size as usize];
            r.read_exact(&mut buf)?;
            PropertyValue::Raw(buf)
        }
        _ => unreachable!(),
    };

    if !matches!(
        prop_type.as_str(),
        "ArrayProperty" | "StructProperty" | "StrProperty" | "DelegateProperty" | "MapProperty"
    ) && prop_type != "BoolProperty"
    {
        let consumed = (r.position() - value_start) as i32;
        if consumed != size {
            r.seek(SeekFrom::Start(value_start + size as u64))?;
        }
    }

    Ok(Some(Property {
        name: prop_name,
        prop_type,
        size,
        array_index,
        value,
        enum_name,
        struct_name,
    }))
}

fn parse_array_ctx(
    r: &mut Cursor<&Vec<u8>>,
    ctx: &PropertyCtx,
    size: i32,
    prop_name: &str,
) -> Result<PropertyValue> {
    let count = r.read_i32::<LittleEndian>()?;
    if count <= 0 {
        let consumed = 4i64;
        let remain = size as i64 - consumed;
        if remain > 0 {
            r.seek(SeekFrom::Current(remain))?;
        }
        return Ok(PropertyValue::Array(Vec::new()));
    }

    if let (Some(db), Some(owner)) = (ctx.db, &ctx.owner) {
        if let Ok(Some((inner_ref, inner_entry))) = db.array_inner_for(owner, prop_name) {
            let mut elems = Vec::with_capacity(count as usize);
            for _ in 0..count {
                elems.push(read_one_by_inner(r, ctx, &inner_ref, &inner_entry)?);
            }
            return Ok(PropertyValue::Array(elems));
        }
    }

    let body = (size as u64).saturating_sub(4);
    let mut buf = vec![0u8; body as usize];
    r.read_exact(&mut buf)?;
    if ctx.db.is_none() {
        eprintln!(
            "  \x1b[33marr\x1b[0m '{prop_name}': no schema (--game-root); \
             {count} elements emitted as Raw"
        );
    } else {
        eprintln!(
            "  \x1b[33marr\x1b[0m '{prop_name}': schema lookup failed; \
             {count} elements emitted as Raw"
        );
    }
    Ok(PropertyValue::Raw(buf))
}

fn read_one_by_inner(
    r: &mut Cursor<&Vec<u8>>,
    ctx: &PropertyCtx,
    _inner_ref: &ResolvedRef,
    inner: &SchemaEntry,
) -> Result<PropertyValue> {
    let kind = match inner {
        SchemaEntry::Property(k) => k,
        _ => {
            return Err(Error::new(
                ErrorKind::InvalidData,
                "array Inner is not a UProperty",
            ));
        }
    };
    Ok(match kind {
        PropertyKind::Byte { .. } => PropertyValue::Byte(r.read_u8()?),
        PropertyKind::Int { .. } => PropertyValue::Int(r.read_i32::<LittleEndian>()?),
        PropertyKind::Bool { .. } => PropertyValue::Bool(r.read_u8()? != 0),
        PropertyKind::Float { .. } => PropertyValue::Float(r.read_f32::<LittleEndian>()?),
        PropertyKind::Object { .. }
        | PropertyKind::Class { .. }
        | PropertyKind::Component { .. }
        | PropertyKind::Interface { .. } => PropertyValue::Object(r.read_i32::<LittleEndian>()?),
        PropertyKind::Name { .. } => PropertyValue::Name(read_fname(r)?),
        PropertyKind::Str { .. } => PropertyValue::String(read_string(r)?),
        PropertyKind::Delegate { .. } => {
            let obj = r.read_i32::<LittleEndian>()?;
            let fnf = read_fname(r)?;
            PropertyValue::AtomicStruct(vec![
                ("Object".into(), PropertyValue::Object(obj)),
                ("Function".into(), PropertyValue::Name(fnf)),
            ])
        }
        PropertyKind::Array { .. } => {
            let cnt = r.read_i32::<LittleEndian>()?;
            let mut v = Vec::with_capacity(cnt.max(0) as usize);
            for _ in 0..cnt.max(0) {
                v.push(read_one_by_inner(r, ctx, _inner_ref, inner)?);
            }
            PropertyValue::Array(v)
        }
        PropertyKind::Map { .. } => {
            return Err(Error::new(
                ErrorKind::Unsupported,
                "TMap inside TArray is not supported",
            ));
        }
        PropertyKind::Struct { struct_obj, .. } => {
            let owner_pkg = ctx.pak;

            let (struct_ref, sentry) = resolve_struct_obj(ctx, *struct_obj)?;
            read_struct_value(r, ctx, &struct_ref, &sentry, owner_pkg)?
        }
    })
}

fn parse_struct_ctx(
    r: &mut Cursor<&Vec<u8>>,
    ctx: &PropertyCtx,
    size: i32,
    struct_name: &str,
    prop_name: &str,
) -> Result<PropertyValue> {
    if is_builtin_atomic(struct_name) {
        return read_builtin_atomic(r, struct_name);
    }

    if let (Some(db), Some(owner)) = (ctx.db, &ctx.owner) {
        if let Ok(Some((sref, sentry))) = db.struct_for(owner, prop_name) {
            let start = r.position();
            let end = start + size as u64;
            let v = read_struct_value(r, ctx, &sref, &sentry, ctx.pak)?;
            if r.position() < end {
                r.seek(SeekFrom::Start(end))?;
            } else if r.position() > end {
                eprintln!(
                    "  \x1b[33mstruct\x1b[0m '{prop_name}' ({struct_name}): \
                     overran by {} bytes",
                    r.position() - end
                );
            }
            return Ok(v);
        }

        if let Ok(Some((sref, sentry))) = db.lookup_struct_by_name(&owner.stem_lc, struct_name) {
            let start = r.position();
            let end = start + size as u64;
            let v = read_struct_value(r, ctx, &sref, &sentry, ctx.pak)?;
            if r.position() < end {
                r.seek(SeekFrom::Start(end))?;
            }
            return Ok(v);
        }
    }

    let start = r.position();
    let end = start + size as u64;
    let mut fields: Vec<Property> = Vec::new();
    let mut ok = true;
    loop {
        if r.position() >= end {
            break;
        }
        match parse_property_ctx(r, &ctx.drop_owner())? {
            Some(p) if p.name == "None" => break,
            Some(p) => fields.push(p),
            None => {
                ok = false;
                break;
            }
        }
    }
    if ok {
        if r.position() < end {
            r.seek(SeekFrom::Start(end))?;
        }
        Ok(PropertyValue::Struct(fields))
    } else {
        r.seek(SeekFrom::Start(start))?;
        let mut buf = vec![0u8; size as usize];
        r.read_exact(&mut buf)?;
        Ok(PropertyValue::Raw(buf))
    }
}

fn resolve_struct_obj(
    ctx: &PropertyCtx,
    struct_obj: i32,
) -> Result<(ResolvedRef, std::rc::Rc<SchemaEntry>)> {
    let db = ctx
        .db
        .ok_or_else(|| Error::new(ErrorKind::InvalidInput, "no SchemaDb for struct resolution"))?;
    let owner = ctx
        .owner
        .as_ref()
        .ok_or_else(|| Error::new(ErrorKind::InvalidInput, "no owner for struct resolution"))?;
    let pkg = db.open_package(&owner.stem_lc)?;
    let sref = db
        .resolve_index(&pkg, struct_obj)?
        .ok_or_else(|| Error::new(ErrorKind::NotFound, "struct_obj didn't resolve"))?;
    let entry = db.entry(&sref)?;
    Ok((sref, entry))
}

fn read_struct_value(
    r: &mut Cursor<&Vec<u8>>,
    ctx: &PropertyCtx,
    sref: &ResolvedRef,
    sentry: &SchemaEntry,
    _owner_pak: &UPKPak,
) -> Result<PropertyValue> {
    let immutable = matches!(
        sentry,
        SchemaEntry::ScriptStruct { struct_flags, .. }
            if (struct_flags & (STRUCT_IMMUTABLE | STRUCT_IMMUTABLE_WHEN_COOKED)) != 0
    );
    let child_ctx = ctx.with_owner(sref.clone());

    if immutable {
        let db = ctx.db.unwrap();
        let kids = db.list_children(sref).unwrap_or_default();
        let mut fields = Vec::with_capacity(kids.len());
        for (name, _cref, centry) in &kids {
            let v = read_one_by_inner(r, &child_ctx, _cref, centry)?;
            fields.push((name.clone(), v));
        }
        Ok(PropertyValue::AtomicStruct(fields))
    } else {
        let mut fields = Vec::new();
        loop {
            match parse_property_ctx(r, &child_ctx)? {
                Some(p) if p.name == "None" => break,
                Some(p) => fields.push(p),
                None => break,
            }
        }
        Ok(PropertyValue::Struct(fields))
    }
}

pub const STRUCT_IMMUTABLE: u32 = 0x00000020;
pub const STRUCT_IMMUTABLE_WHEN_COOKED: u32 = 0x00000080;
pub const STRUCT_ATOMIC: u32 = 0x00000010;

fn read_builtin_atomic(r: &mut Cursor<&Vec<u8>>, name: &str) -> Result<PropertyValue> {
    let mk = |f: Vec<(&str, PropertyValue)>| {
        PropertyValue::AtomicStruct(f.into_iter().map(|(n, v)| (n.to_string(), v)).collect())
    };
    Ok(match name {
        "Guid" => mk(vec![
            ("A", PropertyValue::Int(r.read_i32::<LittleEndian>()?)),
            ("B", PropertyValue::Int(r.read_i32::<LittleEndian>()?)),
            ("C", PropertyValue::Int(r.read_i32::<LittleEndian>()?)),
            ("D", PropertyValue::Int(r.read_i32::<LittleEndian>()?)),
        ]),
        "Vector" => mk(vec![
            ("X", PropertyValue::Float(r.read_f32::<LittleEndian>()?)),
            ("Y", PropertyValue::Float(r.read_f32::<LittleEndian>()?)),
            ("Z", PropertyValue::Float(r.read_f32::<LittleEndian>()?)),
        ]),
        "Vector2D" => mk(vec![
            ("X", PropertyValue::Float(r.read_f32::<LittleEndian>()?)),
            ("Y", PropertyValue::Float(r.read_f32::<LittleEndian>()?)),
        ]),
        "Vector4" | "Quat" | "Plane" => mk(vec![
            ("X", PropertyValue::Float(r.read_f32::<LittleEndian>()?)),
            ("Y", PropertyValue::Float(r.read_f32::<LittleEndian>()?)),
            ("Z", PropertyValue::Float(r.read_f32::<LittleEndian>()?)),
            ("W", PropertyValue::Float(r.read_f32::<LittleEndian>()?)),
        ]),
        "Rotator" => mk(vec![
            ("Pitch", PropertyValue::Int(r.read_i32::<LittleEndian>()?)),
            ("Yaw", PropertyValue::Int(r.read_i32::<LittleEndian>()?)),
            ("Roll", PropertyValue::Int(r.read_i32::<LittleEndian>()?)),
        ]),
        "Color" => mk(vec![
            ("B", PropertyValue::Byte(r.read_u8()?)),
            ("G", PropertyValue::Byte(r.read_u8()?)),
            ("R", PropertyValue::Byte(r.read_u8()?)),
            ("A", PropertyValue::Byte(r.read_u8()?)),
        ]),
        "LinearColor" => mk(vec![
            ("R", PropertyValue::Float(r.read_f32::<LittleEndian>()?)),
            ("G", PropertyValue::Float(r.read_f32::<LittleEndian>()?)),
            ("B", PropertyValue::Float(r.read_f32::<LittleEndian>()?)),
            ("A", PropertyValue::Float(r.read_f32::<LittleEndian>()?)),
        ]),
        "IntPoint" => mk(vec![
            ("X", PropertyValue::Int(r.read_i32::<LittleEndian>()?)),
            ("Y", PropertyValue::Int(r.read_i32::<LittleEndian>()?)),
        ]),
        "Box" => {
            let v = mk(vec![
                ("MinX", PropertyValue::Float(r.read_f32::<LittleEndian>()?)),
                ("MinY", PropertyValue::Float(r.read_f32::<LittleEndian>()?)),
                ("MinZ", PropertyValue::Float(r.read_f32::<LittleEndian>()?)),
                ("MaxX", PropertyValue::Float(r.read_f32::<LittleEndian>()?)),
                ("MaxY", PropertyValue::Float(r.read_f32::<LittleEndian>()?)),
                ("MaxZ", PropertyValue::Float(r.read_f32::<LittleEndian>()?)),
                ("IsValid", PropertyValue::Byte(r.read_u8()?)),
            ]);
            v
        }
        "Box2D" => mk(vec![
            ("MinX", PropertyValue::Float(r.read_f32::<LittleEndian>()?)),
            ("MinY", PropertyValue::Float(r.read_f32::<LittleEndian>()?)),
            ("MaxX", PropertyValue::Float(r.read_f32::<LittleEndian>()?)),
            ("MaxY", PropertyValue::Float(r.read_f32::<LittleEndian>()?)),
            ("IsValid", PropertyValue::Byte(r.read_u8()?)),
        ]),
        _ => {
            return Err(Error::new(
                ErrorKind::Unsupported,
                format!("builtin atomic '{name}' not in fast-path table"),
            ));
        }
    })
}

pub fn parse_array(
    r: &mut Cursor<&Vec<u8>>,
    pak: &UPKPak,
    size: i32,
    ver: i16,
) -> Result<PropertyValue> {
    parse_array_ctx(r, &PropertyCtx::legacy(pak, ver), size, "")
}

pub fn parse_struct(
    r: &mut Cursor<&Vec<u8>>,
    pak: &UPKPak,
    size: i32,
    struct_name: &str,
    ver: i16,
) -> Result<PropertyValue> {
    parse_struct_ctx(r, &PropertyCtx::legacy(pak, ver), size, struct_name, "")
}
