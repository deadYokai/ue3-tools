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

fn read_count(r: &mut Cursor<&Vec<u8>>) -> Result<i32> {
    let count = r.read_i32::<LittleEndian>()?;
    let remaining = (r.get_ref().len() as u64).saturating_sub(r.position());
    if count < 0 || count as u64 > remaining {
        return Err(Error::new(
            ErrorKind::InvalidData,
            format!("array count {count} implausible vs {remaining} remaining bytes"),
        ));
    }
    Ok(count)
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
    let value_start = r.position();
    let blob_len = {
        let e = r.seek(SeekFrom::End(0))?;
        r.seek(SeekFrom::Start(value_start))?;
        e
    };
    let end = value_start.saturating_add(size.max(0) as u64).min(blob_len);

    let count = read_count(r)?;
    if count <= 0 {
        if r.position() < end {
            r.seek(SeekFrom::Start(end))?;
        }
        return Ok(PropertyValue::Array(Vec::new()));
    }

    if let (Some(db), Some(owner)) = (ctx.db, &ctx.owner) {
        if let Ok(Some((inner_ref, inner_entry))) = db.array_inner_for(owner, prop_name) {
            if let SchemaEntry::Property(PropertyKind::Struct { struct_obj, .. }) = &*inner_entry {
                let body_start = r.position();
                let elem_ctx = ctx.with_owner(inner_ref.clone());
                if let Ok((struct_ref, _)) = resolve_struct_obj(&elem_ctx, *struct_obj) {
                    let mut bin_elems = Vec::with_capacity(count as usize);
                    let mut bin_ok = true;
                    for _ in 0..count {
                        match read_struct_binary(r, &elem_ctx, &struct_ref) {
                            Ok(v) => bin_elems.push(v),
                            Err(_) => {
                                bin_ok = false;
                                break;
                            }
                        }
                        if r.position() > end {
                            bin_ok = false;
                            break;
                        }
                    }
                    if bin_ok && r.position() == end {
                        return Ok(PropertyValue::Array(bin_elems));
                    }
                    r.seek(SeekFrom::Start(body_start))?;
                }
            }
            let mut elems = Vec::with_capacity(count as usize);
            let mut errored = false;
            for _ in 0..count {
                match read_one_by_inner(r, ctx, &inner_ref, &inner_entry) {
                    Ok(v) => elems.push(v),
                    Err(_) => {
                        errored = true;
                        break;
                    }
                }
                if r.position() > end {
                    errored = true;
                    break;
                }
            }
            let consumed_exactly = !errored && r.position() == end;
            if r.position() != end {
                r.seek(SeekFrom::Start(end))?;
            }
            if consumed_exactly {
                return Ok(PropertyValue::Array(elems));
            }
            eprintln!(
                "  \x1b[33marr\x1b[0m '{prop_name}': {count} elements did not match \
                 tag size ({size} bytes); emitted as Raw"
            );
            let mut buf = vec![0u8; (end - value_start) as usize];
            r.seek(SeekFrom::Start(value_start))?;
            r.read_exact(&mut buf)?;
            r.seek(SeekFrom::Start(end))?;
            return Ok(PropertyValue::Raw(buf));
        }
    }

    let mut buf = vec![0u8; (end - value_start) as usize];
    r.seek(SeekFrom::Start(value_start))?;
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
    inner_ref: &ResolvedRef,
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
            let cnt = read_count(r)?;
            let mut v = Vec::with_capacity(cnt.max(0) as usize);
            for _ in 0..cnt.max(0) {
                v.push(read_one_by_inner(r, ctx, inner_ref, inner)?);
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
            let elem_ctx = ctx.with_owner(inner_ref.clone());
            let (struct_ref, sentry) = resolve_struct_obj(&elem_ctx, *struct_obj)?;
            read_struct_value(r, &elem_ctx, &struct_ref, &sentry, ctx.pak)?
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
            let end = start + size.max(0) as u64;
            {
                let bin_ctx = ctx.with_owner(sref.clone());
                if let Ok(v) = read_struct_binary(r, &bin_ctx, &sref) {
                    if r.position() == end {
                        return Ok(v);
                    }
                }
                r.seek(SeekFrom::Start(start))?;
            }
            let v = read_struct_value(r, ctx, &sref, &sentry, ctx.pak)?;
            if r.position() > end {
                eprintln!(
                    "  \x1b[33mstruct\x1b[0m '{prop_name}' ({struct_name}): \
                     overran by {} bytes; realigning to tag size",
                    r.position() - end
                );
            }
            if r.position() != end {
                r.seek(SeekFrom::Start(end))?;
            }
            return Ok(v);
        }

        if let Ok(Some((sref, sentry))) = db.lookup_struct_by_name(&owner.stem_lc, struct_name) {
            let start = r.position();
            let end = start + size.max(0) as u64;
            let v = read_struct_value(r, ctx, &sref, &sentry, ctx.pak)?;
            if r.position() != end {
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
    if ctx.db.is_some() && struct_is_binary(sentry) {
        return read_struct_binary(r, ctx, sref);
    }

    let child_ctx = ctx.with_owner(sref.clone());
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

fn struct_is_binary(sentry: &SchemaEntry) -> bool {
    if let SchemaEntry::ScriptStruct { extra, .. } = sentry {
        let f = extra.struct_flags;
        (f & STRUCT_IMMUTABLE) != 0 || (f & STRUCT_IMMUTABLE_WHEN_COOKED) != 0
    } else {
        false
    }
}

fn read_struct_binary(
    r: &mut Cursor<&Vec<u8>>,
    ctx: &PropertyCtx,
    sref: &ResolvedRef,
) -> Result<PropertyValue> {
    let db = match ctx.db {
        Some(d) => d,
        None => {
            return Err(Error::new(
                ErrorKind::InvalidInput,
                "no SchemaDb for binary struct read",
            ));
        }
    };
    let chain = db.class_chain(sref).unwrap_or_else(|_| vec![sref.clone()]);
    let mut fields: Vec<(String, PropertyValue)> = Vec::new();
    for klass in &chain {
        let field_ctx = ctx.with_owner(klass.clone());
        for (name, cref, centry) in db.list_children(klass).unwrap_or_default() {
            let dim = match &*centry {
                SchemaEntry::Property(k) => k.common().array_dim.max(1),
                _ => continue,
            };
            if dim == 1 {
                fields.push((name, read_one_by_inner(r, &field_ctx, &cref, &centry)?));
            } else {
                let mut arr = Vec::with_capacity(dim as usize);
                for _ in 0..dim {
                    arr.push(read_one_by_inner(r, &field_ctx, &cref, &centry)?);
                }
                fields.push((name, PropertyValue::Array(arr)));
            }
        }
    }
    Ok(PropertyValue::AtomicStruct(fields))
}

pub const CPF_NATIVE: u64 = 0x0000_0000_0000_1000;

type NativeField = (ResolvedRef, String, ResolvedRef, std::rc::Rc<SchemaEntry>);

pub fn read_native_props(
    tail: &[u8],
    ctx_pak: &UPKPak,
    ver: i16,
    db: &SchemaDb,
    class_ref: &ResolvedRef,
) -> Option<Vec<Property>> {
    if tail.is_empty() {
        return None;
    }
    let chain = db.class_chain(class_ref).ok()?; // [self, super, ..., root]
    let collect = |classes: &[ResolvedRef]| -> Vec<NativeField> {
        let mut v = Vec::new();
        for k in classes {
            for (name, pref, pentry) in db.list_children(k).unwrap_or_default() {
                let native = matches!(&*pentry, SchemaEntry::Property(pk)
                    if pk.common().property_flags & CPF_NATIVE != 0);
                if native {
                    v.push((k.clone(), name, pref, pentry));
                }
            }
        }
        v
    };

    let mut base_first: Vec<ResolvedRef> = chain.clone();
    base_first.reverse();

    let candidates = [collect(&chain), collect(&base_first), collect(&chain[..1])];

    let mut best: (usize, u64) = (0, 0);
    for list in &candidates {
        if list.is_empty() {
            continue;
        }
        match try_read_native_list(tail, ctx_pak, ver, db, list) {
            Ok(fields) => return Some(fields),
            Err(miss) => {
                if miss.1 > best.1 {
                    best = miss;
                }
            }
        }
    }

    if best.0 > 0 {
        eprintln!(
            "  \x1b[33mnative\x1b[0m '{}': no CPF_Native ordering consumed the tail \
             exactly (best {} field(s), {} of {} bytes); emitting Raw",
            db.export_object_name(class_ref).unwrap_or_default(),
            best.0,
            best.1,
            tail.len()
        );
    }
    None
}

fn try_read_native_list(
    tail: &[u8],
    ctx_pak: &UPKPak,
    ver: i16,
    db: &SchemaDb,
    list: &[NativeField],
) -> std::result::Result<Vec<Property>, (usize, u64)> {
    let blob = tail.to_vec();
    let mut r = Cursor::new(&blob);
    let mut out: Vec<Property> = Vec::new();

    for (kref, name, pref, pentry) in list {
        let kctx = PropertyCtx {
            pak: ctx_pak,
            ver,
            db: Some(db),
            owner: Some(kref.clone()),
        };
        let dim = match &**pentry {
            SchemaEntry::Property(k) => k.common().array_dim.max(1),
            _ => 1,
        };
        let value = if dim > 1 {
            let mut arr = Vec::with_capacity(dim as usize);
            for _ in 0..dim {
                match read_value_positional(&mut r, &kctx, pref, pentry) {
                    Ok(v) => arr.push(v),
                    Err(_) => return Err((out.len(), r.position())),
                }
            }
            PropertyValue::Array(arr)
        } else {
            match read_value_positional(&mut r, &kctx, pref, pentry) {
                Ok(v) => v,
                Err(_) => return Err((out.len(), r.position())),
            }
        };
        out.push(Property {
            name: name.clone(),
            prop_type: String::new(),
            size: 0,
            array_index: 0,
            value,
            enum_name: None,
            struct_name: None,
        });
        if r.position() as usize > blob.len() {
            return Err((out.len(), r.position()));
        }
    }

    if r.position() as usize == blob.len() {
        Ok(out)
    } else {
        Err((out.len(), r.position()))
    }
}

fn native_tail_miss(out: &[Property], consumed: u64, total: usize) -> Option<Vec<Property>> {
    if !out.is_empty() {
        eprintln!(
            "  \x1b[33mnative\x1b[0m schema parse read {} CPF_Native field(s) but \
             consumed {} of {} tail bytes; emitting Raw",
            out.len(),
            consumed,
            total
        );
    }
    None
}

fn read_value_positional(
    r: &mut Cursor<&Vec<u8>>,
    ctx: &PropertyCtx,
    prop_ref: &ResolvedRef,
    entry: &SchemaEntry,
) -> Result<PropertyValue> {
    let kind = match entry {
        SchemaEntry::Property(k) => k,
        _ => {
            return Err(Error::new(
                ErrorKind::InvalidData,
                "native field is not a UProperty",
            ));
        }
    };
    match kind {
        PropertyKind::Struct { struct_obj, .. } => {
            let sctx = ctx.with_owner(prop_ref.clone());
            let (sref, _) = resolve_struct_obj(&sctx, *struct_obj)?;
            read_struct_binary(r, &sctx, &sref)
        }
        PropertyKind::Array { inner, .. } => {
            let count = read_count(r)?;
            let db = ctx
                .db
                .ok_or_else(|| Error::new(ErrorKind::InvalidInput, "no SchemaDb"))?;
            let pkg = db.open_package(&prop_ref.stem_lc)?;
            let inner_ref = db
                .resolve_index(&pkg, *inner)?
                .ok_or_else(|| Error::new(ErrorKind::NotFound, "array inner unresolved"))?;
            let inner_entry = db.entry(&inner_ref)?;
            let mut v = Vec::with_capacity(count.max(0) as usize);
            for _ in 0..count.max(0) {
                v.push(read_value_positional(r, ctx, &inner_ref, &inner_entry)?);
            }
            Ok(PropertyValue::Array(v))
        }
        _ => read_one_by_inner(r, ctx, prop_ref, entry),
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
