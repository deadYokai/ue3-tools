use std::{
    collections::HashMap,
    fmt::Write as FmtWrite,
    io::{Result, Write},
    path::{Path, PathBuf},
};

use crate::{
    native::{Mip, MipSource, NativePayload},
    schema::{PropertyKind, SchemaEntry},
    schemadb::{ResolvedRef, SchemaDb},
    upkprops::{Property, PropertyValue},
    upkreader::{FName, UPKPak},
};

const INDENT: &str = "    ";

struct RefResolver<'a> {
    pak: &'a UPKPak,
    by_name: HashMap<String, Vec<i32>>,
}

impl<'a> RefResolver<'a> {
    fn new(pak: &'a UPKPak) -> Self {
        let mut by_name: HashMap<String, Vec<i32>> = HashMap::new();
        for (i, exp) in pak.export_table.iter().enumerate() {
            let n = pak.fname_to_string(&exp.object_name);
            by_name.entry(n).or_default().push((i + 1) as i32);
        }
        Self { pak, by_name }
    }

    fn label_for_index(&self, idx: i32) -> String {
        if idx == 0 {
            return "None".into();
        }
        if idx > 0 {
            let exp = match self.pak.export_table.get((idx - 1) as usize) {
                Some(e) => e,
                None => return format!("<invalid export #{idx}>"),
            };
            let name = self.pak.fname_to_string(&exp.object_name);
            let ambiguous = self
                .by_name
                .get(&name)
                .map(|v| v.len() > 1)
                .unwrap_or(false);
            if !ambiguous {
                name
            } else {
                let outer = self.outer_label(exp.outer_index);
                if outer.is_empty() {
                    name
                } else {
                    format!("{outer}.{name}")
                }
            }
        } else {
            let imp = match self.pak.import_table.get((-idx - 1) as usize) {
                Some(i) => i,
                None => return format!("<invalid import #{idx}>"),
            };
            let name = self.pak.fname_to_string(&imp.object_name);
            let top = self.import_top_pkg(idx);
            match top {
                Some(t) if t != name => format!("extern:{t}::{name}"),
                _ => format!("extern:{name}"),
            }
        }
    }

    fn outer_label(&self, outer_idx: i32) -> String {
        let mut parts = Vec::new();
        let mut cur = outer_idx;
        let mut guard = 0;
        while cur > 0 && guard < 16 {
            guard += 1;
            let exp = match self.pak.export_table.get((cur - 1) as usize) {
                Some(e) => e,
                None => break,
            };
            parts.push(self.pak.fname_to_string(&exp.object_name));
            cur = exp.outer_index;
        }
        parts.reverse();
        parts.join(".")
    }

    fn import_top_pkg(&self, idx: i32) -> Option<String> {
        let mut cur = idx;
        let mut guard = 0;
        while cur < 0 {
            guard += 1;
            if guard > 64 {
                return None;
            }
            let imp = self.pak.import_table.get((-cur - 1) as usize)?;
            if imp.outer_index == 0 {
                return Some(self.pak.fname_to_string(&imp.object_name));
            }
            cur = imp.outer_index;
        }
        None
    }
}

pub struct EmitInput<'a> {
    pub class_name: &'a str,
    pub export_short_name: &'a str,
    pub export_full_path: &'a str,
    pub export_index: i32,
    pub net_index: Option<i32>,
    pub props: &'a [Property],
    pub consumed_props: &'a [String],
    pub payload: &'a NativePayload,
    pub sidecars: &'a [PathBuf],
    pub pak: &'a UPKPak,
    pub pkg_stem: &'a str,
    pub p_ver: i16,
}

pub fn write_uo_file(path: &Path, input: &EmitInput) -> Result<()> {
    let s = render(input);
    let mut f = std::fs::File::create(path)?;
    f.write_all(s.as_bytes())?;
    Ok(())
}

pub fn render(input: &EmitInput) -> String {
    let resolver = RefResolver::new(input.pak);
    let mut out = String::new();

    let _ = writeln!(
        out,
        "// ue3-tools  pkg={}.upk  p_ver={}  export=#{}",
        input.pkg_stem, input.p_ver, input.export_index
    );
    let _ = writeln!(out, "// path: {}", input.export_full_path);
    if let Some(n) = input.net_index {
        let _ = writeln!(out, "// net_index: {n}");
    }
    out.push('\n');

    let _ = writeln!(out, "{} {} {{", input.class_name, input.export_short_name);

    let consumed: std::collections::HashSet<&str> =
        input.consumed_props.iter().map(String::as_str).collect();

    for p in input.props {
        if p.name == "None" {
            continue;
        }
        if consumed.contains(p.name.as_str()) {
            let _ = writeln!(
                out,
                "{INDENT}{} = @sidecar  // externalized by {}",
                p.name,
                input.payload.variant_label()
            );
            continue;
        }
        let mut line = String::new();
        let _ = write!(line, "{INDENT}{} = ", p.name);
        render_value(&mut line, &p.value, &resolver, input.pak, 1);
        line.push('\n');
        out.push_str(&line);
    }

    match input.payload {
        NativePayload::Empty { .. } => {}
        NativePayload::NativeProps { fields } => {
            if !fields.is_empty() {
                out.push('\n');
                let _ = writeln!(out, "{INDENT}// native-serialized (CPF_Native)");
                for p in fields {
                    let mut line = String::new();
                    let _ = write!(line, "{INDENT}{} = ", p.name);
                    render_value(&mut line, &p.value, &resolver, input.pak, 1);
                    line.push('\n');
                    out.push_str(&line);
                }
            }
        }
        _ => {
            out.push('\n');
            render_native(&mut out, input.payload, input.class_name, 1);
        }
    }

    if !input.sidecars.is_empty() {
        out.push('\n');
        for s in input.sidecars {
            let fname = s.file_name().and_then(|n| n.to_str()).unwrap_or("?");
            let _ = writeln!(out, "{INDENT}@sidecar = \"{fname}\"");
        }
    }

    out.push_str("}\n");
    out
}

pub fn render_class_def(
    db: &SchemaDb,
    self_ref: &ResolvedRef,
    pak: &UPKPak,
    pkg_stem: &str,
    p_ver: i16,
    export_index: i32,
    export_full_path: &str,
    cdo_props: &[Property],
) -> Option<String> {
    let entry = db.entry(self_ref).ok()?;
    let (header, extra) = match &*entry {
        SchemaEntry::Class { header, extra, .. } => (header, extra),
        _ => return None,
    };

    let class_name = db
        .export_object_name(self_ref)
        .or_else(|| export_full_path.split_once(' ').map(|(_, n)| n.to_string()))
        .unwrap_or_else(|| "Class".to_string());

    let mut out = String::new();
    let _ = writeln!(
        out,
        "// ue3-tools  pkg={}.upk  p_ver={}  export=#{}",
        pkg_stem, p_ver, export_index
    );
    let _ = writeln!(out, "// path: {}", export_full_path);
    let cfg = pak.fname_to_string(&extra.class_config_name);
    let _ = writeln!(
        out,
        "// class_flags=0x{:08x}  within={}  config={}",
        extra.class_flags,
        leaf_name(pak, extra.class_within),
        if cfg.is_empty() { "None" } else { cfg.as_str() }
    );
    out.push('\n');

    if header.super_struct != 0 {
        let _ = writeln!(
            out,
            "class {} extends {} {{",
            class_name,
            leaf_name(pak, header.super_struct)
        );
    } else {
        let _ = writeln!(out, "class {} {{", class_name);
    }

    let children = db.list_children(self_ref).unwrap_or_default();
    let mut consts: Vec<(String, String)> = Vec::new();
    let mut enums: Vec<(String, Vec<String>)> = Vec::new();
    let mut structs: Vec<(String, ResolvedRef, u32)> = Vec::new();
    let mut vars: Vec<(String, std::rc::Rc<SchemaEntry>)> = Vec::new();
    let mut funcs: Vec<String> = Vec::new();
    let mut states: Vec<String> = Vec::new();

    for (cname, cref, centry) in &children {
        match &**centry {
            SchemaEntry::Property(_) => vars.push((cname.clone(), centry.clone())),
            SchemaEntry::Enum { names, .. } => {
                let vals = names.iter().map(|f| pak.fname_to_string(f)).collect();
                enums.push((cname.clone(), vals));
            }
            SchemaEntry::ScriptStruct { extra, .. } => {
                structs.push((cname.clone(), cref.clone(), extra.struct_flags));
            }
            SchemaEntry::Const { value, .. } => consts.push((cname.clone(), value.clone())),
            SchemaEntry::Function { .. } => funcs.push(cname.clone()),
            SchemaEntry::State { .. } => states.push(cname.clone()),
            _ => {}
        }
    }

    let mut wrote = false;

    if !consts.is_empty() {
        for (n, v) in &consts {
            let _ = writeln!(out, "{INDENT}const {} = {};", n, v);
        }
        wrote = true;
    }

    if !enums.is_empty() {
        if wrote {
            out.push('\n');
        }
        for (n, vals) in &enums {
            let _ = writeln!(out, "{INDENT}enum {} {{", n);
            for v in vals {
                let _ = writeln!(out, "{INDENT}{INDENT}{},", v);
            }
            let _ = writeln!(out, "{INDENT}}};");
        }
        wrote = true;
    }

    if !structs.is_empty() {
        if wrote {
            out.push('\n');
        }
        for (n, sref, flags) in &structs {
            render_struct_decl(&mut out, db, pak, &self_ref.stem_lc, n, sref, *flags, 1);
        }
        wrote = true;
    }

    if !vars.is_empty() {
        if wrote {
            out.push('\n');
        }
        for (n, centry) in &vars {
            if let SchemaEntry::Property(k) = &**centry {
                render_var(&mut out, db, pak, &self_ref.stem_lc, n, k, 1);
            }
        }
        wrote = true;
    }

    if !funcs.is_empty() {
        if wrote {
            out.push('\n');
        }
        for f in &funcs {
            let _ = writeln!(out, "{INDENT}function {}();   // body omitted", f);
        }
        wrote = true;
    }

    if !states.is_empty() {
        if wrote {
            out.push('\n');
        }
        for s in &states {
            let _ = writeln!(out, "{INDENT}// state {}", s);
        }
        wrote = true;
    }

    if cdo_props.iter().any(|p| p.name != "None") {
        if wrote {
            out.push('\n');
        }
        let resolver = RefResolver::new(pak);
        let _ = writeln!(out, "{INDENT}defaultproperties {{");
        for p in cdo_props {
            if p.name == "None" {
                continue;
            }
            let mut line = String::new();
            let _ = write!(line, "{INDENT}{INDENT}{} = ", p.name);
            render_value(&mut line, &p.value, &resolver, pak, 2);
            line.push('\n');
            out.push_str(&line);
        }
        let _ = writeln!(out, "{INDENT}}}");
    }

    out.push_str("}\n");
    Some(out)
}

fn def_header(out: &mut String, pkg_stem: &str, p_ver: i16, export_index: i32, full_path: &str) {
    let _ = writeln!(
        out,
        "// ue3-tools  pkg={}.upk  p_ver={}  export=#{}",
        pkg_stem, p_ver, export_index
    );
    let _ = writeln!(out, "// path: {}", full_path);
}

fn def_name(db: &SchemaDb, self_ref: &ResolvedRef, full_path: &str, fallback: &str) -> String {
    db.export_object_name(self_ref)
        .or_else(|| full_path.split_once(' ').map(|(_, n)| n.to_string()))
        .unwrap_or_else(|| fallback.to_string())
}

pub fn render_struct_def(
    db: &SchemaDb,
    self_ref: &ResolvedRef,
    pak: &UPKPak,
    pkg_stem: &str,
    p_ver: i16,
    export_index: i32,
    export_full_path: &str,
    default_props: &[Property],
) -> Option<String> {
    let entry = db.entry(self_ref).ok()?;
    let flags = match &*entry {
        SchemaEntry::ScriptStruct { extra, .. } => extra.struct_flags,
        SchemaEntry::Struct { .. } => 0,
        _ => return None,
    };
    let name = def_name(db, self_ref, export_full_path, "Struct");

    let mut out = String::new();
    def_header(&mut out, pkg_stem, p_ver, export_index, export_full_path);
    let _ = writeln!(out, "// struct_flags=0x{:08x}", flags);
    out.push('\n');

    let _ = writeln!(out, "struct {}{} {{", struct_mods(flags), name);
    render_struct_body(&mut out, db, pak, &self_ref.stem_lc, self_ref, 1);

    if default_props.iter().any(|p| p.name != "None") {
        out.push('\n');
        let resolver = RefResolver::new(pak);
        let _ = writeln!(out, "{INDENT}structdefaultproperties {{");
        for p in default_props {
            if p.name == "None" {
                continue;
            }
            let mut line = String::new();
            let _ = write!(line, "{INDENT}{INDENT}{} = ", p.name);
            render_value(&mut line, &p.value, &resolver, pak, 2);
            line.push('\n');
            out.push_str(&line);
        }
        let _ = writeln!(out, "{INDENT}}}");
    }

    out.push_str("}\n");
    Some(out)
}

pub fn render_enum_def(
    db: &SchemaDb,
    self_ref: &ResolvedRef,
    pak: &UPKPak,
    pkg_stem: &str,
    p_ver: i16,
    export_index: i32,
    export_full_path: &str,
) -> Option<String> {
    let entry = db.entry(self_ref).ok()?;
    let names = match &*entry {
        SchemaEntry::Enum { names, .. } => names,
        _ => return None,
    };
    let name = def_name(db, self_ref, export_full_path, "Enum");

    let mut out = String::new();
    def_header(&mut out, pkg_stem, p_ver, export_index, export_full_path);
    out.push('\n');
    let _ = writeln!(out, "enum {} {{", name);
    for v in names {
        let _ = writeln!(out, "{INDENT}{},", pak.fname_to_string(v));
    }
    out.push_str("};\n");
    Some(out)
}

pub fn render_property_def(
    db: &SchemaDb,
    self_ref: &ResolvedRef,
    pak: &UPKPak,
    pkg_stem: &str,
    p_ver: i16,
    export_index: i32,
    export_full_path: &str,
) -> Option<String> {
    let entry = db.entry(self_ref).ok()?;
    let kind = match &*entry {
        SchemaEntry::Property(k) => k,
        _ => return None,
    };
    let name = def_name(db, self_ref, export_full_path, "prop");

    let mut out = String::new();
    def_header(&mut out, pkg_stem, p_ver, export_index, export_full_path);
    out.push('\n');

    let ty = type_of(db, pak, &self_ref.stem_lc, kind);
    let common = kind.common();
    let dim = if common.array_dim > 1 {
        format!("[{}]", common.array_dim)
    } else {
        String::new()
    };
    if common.property_flags != 0 {
        let _ = writeln!(
            out,
            "var {} {}{};   // flags=0x{:016x}",
            ty, name, dim, common.property_flags
        );
    } else {
        let _ = writeln!(out, "var {} {}{};", ty, name, dim);
    }
    Some(out)
}

pub fn render_const_def(
    db: &SchemaDb,
    self_ref: &ResolvedRef,
    _pak: &UPKPak,
    pkg_stem: &str,
    p_ver: i16,
    export_index: i32,
    export_full_path: &str,
) -> Option<String> {
    let entry = db.entry(self_ref).ok()?;
    let value = match &*entry {
        SchemaEntry::Const { value, .. } => value.clone(),
        _ => return None,
    };
    let name = def_name(db, self_ref, export_full_path, "CONST");

    let mut out = String::new();
    def_header(&mut out, pkg_stem, p_ver, export_index, export_full_path);
    out.push('\n');
    let _ = writeln!(out, "const {} = {};", name, value);
    Some(out)
}

fn leaf_name(pak: &UPKPak, idx: i32) -> String {
    if idx > 0 {
        pak.export_table
            .get((idx - 1) as usize)
            .map(|e| pak.fname_to_string(&e.object_name))
            .unwrap_or_else(|| format!("<export#{idx}>"))
    } else if idx < 0 {
        pak.import_table
            .get((-idx - 1) as usize)
            .map(|i| pak.fname_to_string(&i.object_name))
            .unwrap_or_else(|| format!("<import#{idx}>"))
    } else {
        "None".to_string()
    }
}

fn type_of(db: &SchemaDb, pak: &UPKPak, stem_lc: &str, kind: &PropertyKind) -> String {
    use PropertyKind::*;
    match kind {
        Int { .. } => "int".to_string(),
        Float { .. } => "float".to_string(),
        Bool { .. } => "bool".to_string(),
        Str { .. } => "string".to_string(),
        Name { .. } => "name".to_string(),
        Byte { enum_obj, .. } => {
            if *enum_obj != 0 {
                leaf_name(pak, *enum_obj)
            } else {
                "byte".to_string()
            }
        }
        Object { property_class, .. } => {
            if *property_class != 0 {
                leaf_name(pak, *property_class)
            } else {
                "Object".to_string()
            }
        }
        Component { property_class, .. } => {
            if *property_class != 0 {
                leaf_name(pak, *property_class)
            } else {
                "Component".to_string()
            }
        }
        Interface {
            interface_class, ..
        } => {
            if *interface_class != 0 {
                leaf_name(pak, *interface_class)
            } else {
                "Interface".to_string()
            }
        }
        Class { meta_class, .. } => {
            if *meta_class != 0 {
                format!("class<{}>", leaf_name(pak, *meta_class))
            } else {
                "class".to_string()
            }
        }
        Delegate { function, .. } => {
            if *function != 0 {
                format!("delegate<{}>", leaf_name(pak, *function))
            } else {
                "delegate".to_string()
            }
        }
        Array { inner, .. } => format!("array<{}>", inner_type(db, pak, stem_lc, *inner)),
        Map { key, value, .. } => format!(
            "map<{}, {}>",
            inner_type(db, pak, stem_lc, *key),
            inner_type(db, pak, stem_lc, *value)
        ),
        Struct { struct_obj, .. } => {
            if *struct_obj != 0 {
                leaf_name(pak, *struct_obj)
            } else {
                "struct".to_string()
            }
        }
    }
}

fn inner_type(db: &SchemaDb, pak: &UPKPak, stem_lc: &str, idx: i32) -> String {
    if idx <= 0 {
        return leaf_name(pak, idx);
    }
    let r = ResolvedRef {
        stem_lc: stem_lc.to_string(),
        export_idx: idx,
    };
    match db.entry(&r) {
        Ok(e) => match &*e {
            SchemaEntry::Property(k) => type_of(db, pak, stem_lc, k),
            _ => leaf_name(pak, idx),
        },
        Err(_) => leaf_name(pak, idx),
    }
}

fn render_var(
    out: &mut String,
    db: &SchemaDb,
    pak: &UPKPak,
    stem_lc: &str,
    name: &str,
    kind: &PropertyKind,
    depth: usize,
) {
    let pad = INDENT.repeat(depth);
    let ty = type_of(db, pak, stem_lc, kind);
    let dim = kind.common().array_dim;
    if dim > 1 {
        let _ = writeln!(out, "{pad}var {} {}[{}];", ty, name, dim);
    } else {
        let _ = writeln!(out, "{pad}var {} {};", ty, name);
    }
}

fn struct_mods(flags: u32) -> &'static str {
    if flags & 0x20 != 0 {
        "immutable "
    } else if flags & 0x80 != 0 {
        "immutablewhencooked "
    } else {
        ""
    }
}

fn render_struct_body(
    out: &mut String,
    db: &SchemaDb,
    pak: &UPKPak,
    stem_lc: &str,
    sref: &ResolvedRef,
    depth: usize,
) {
    for (cname, cref, centry) in db.list_children(sref).unwrap_or_default() {
        match &*centry {
            SchemaEntry::Property(k) => render_var(out, db, pak, stem_lc, &cname, k, depth),
            SchemaEntry::ScriptStruct { extra, .. } => render_struct_decl(
                out,
                db,
                pak,
                stem_lc,
                &cname,
                &cref,
                extra.struct_flags,
                depth,
            ),
            SchemaEntry::Enum { names, .. } => {
                let pad = INDENT.repeat(depth);
                let _ = writeln!(out, "{pad}enum {} {{", cname);
                for v in names {
                    let _ = writeln!(out, "{pad}{INDENT}{},", pak.fname_to_string(v));
                }
                let _ = writeln!(out, "{pad}}};");
            }
            _ => {}
        }
    }
}

fn render_struct_decl(
    out: &mut String,
    db: &SchemaDb,
    pak: &UPKPak,
    stem_lc: &str,
    name: &str,
    sref: &ResolvedRef,
    flags: u32,
    depth: usize,
) {
    let pad = INDENT.repeat(depth);
    let _ = writeln!(out, "{pad}struct {}{} {{", struct_mods(flags), name);
    render_struct_body(out, db, pak, stem_lc, sref, depth + 1);
    let _ = writeln!(out, "{pad}}};");
}

fn render_value(out: &mut String, v: &PropertyValue, r: &RefResolver, pak: &UPKPak, depth: usize) {
    use PropertyValue::*;
    match v {
        None => out.push_str("None"),
        Byte(b) => {
            let _ = write!(out, "0x{:02x}", b);
        }
        Int(i) => {
            let _ = write!(out, "{i}");
        }
        Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Float(f) => {
            if f.fract() == 0.0 && f.is_finite() {
                let _ = write!(out, "{:.1}", f);
            } else {
                let _ = write!(out, "{}", f);
            }
        }
        Object(idx) => {
            let _ = write!(out, "&{}", r.label_for_index(*idx));
        }
        ObjectRef(s) => {
            let bare = s.splitn(2, ' ').nth(1).unwrap_or(s);
            let _ = write!(out, "&{bare}");
        }
        Name(fn_) => render_fname(out, fn_, pak),
        EnumLabel(s) => out.push_str(s),
        String(s) => {
            let _ = write!(out, "{:?}", s);
        }
        Array(items) => render_array(out, items, r, pak, depth),
        Struct(fields) => render_struct(out, fields, r, pak, depth, Option::None),
        AtomicStruct(fields) => render_atomic_struct(out, fields, r, pak, depth),
        Raw(bytes) => {
            let _ = write!(out, "@bytes({} bytes)  // ", bytes.len());
            for b in bytes.iter().take(16) {
                let _ = write!(out, "{:02x}", b);
            }
            if bytes.len() > 16 {
                out.push_str("…");
            }
        }
    }
}

fn render_fname(out: &mut String, f: &FName, pak: &UPKPak) {
    let n = pak.fname_to_string(f);
    let _ = write!(out, "'{}'", n);
}

fn render_array(
    out: &mut String,
    items: &[PropertyValue],
    r: &RefResolver,
    pak: &UPKPak,
    depth: usize,
) {
    if items.is_empty() {
        out.push_str("[ ]");
        return;
    }

    let scalar_only = items.iter().all(|v| {
        matches!(
            v,
            PropertyValue::Byte(_)
                | PropertyValue::Int(_)
                | PropertyValue::Bool(_)
                | PropertyValue::Float(_)
                | PropertyValue::Name(_)
                | PropertyValue::String(_)
                | PropertyValue::EnumLabel(_)
                | PropertyValue::Object(_)
                | PropertyValue::ObjectRef(_)
                | PropertyValue::None
        )
    });
    if scalar_only && items.len() <= 8 {
        out.push('[');
        for (i, v) in items.iter().enumerate() {
            if i > 0 {
                out.push_str(", ");
            }
            render_value(out, v, r, pak, depth);
        }
        out.push(']');
        return;
    }

    let pad = INDENT.repeat(depth);
    let pad_in = INDENT.repeat(depth + 1);
    out.push_str("[\n");
    for v in items {
        out.push_str(&pad_in);
        render_value(out, v, r, pak, depth + 1);
        out.push('\n');
    }
    out.push_str(&pad);
    out.push(']');
}

fn render_struct(
    out: &mut String,
    fields: &[Property],
    r: &RefResolver,
    pak: &UPKPak,
    depth: usize,
    type_label: Option<&str>,
) {
    if let Some(t) = type_label {
        let _ = write!(out, "{t} ");
    }
    if fields.iter().all(|p| p.name == "None") {
        out.push_str("{ }");
        return;
    }
    let pad = INDENT.repeat(depth);
    let pad_in = INDENT.repeat(depth + 1);
    out.push_str("{\n");
    for p in fields {
        if p.name == "None" {
            continue;
        }
        out.push_str(&pad_in);
        let _ = write!(out, "{} = ", p.name);
        render_value(out, &p.value, r, pak, depth + 1);
        out.push('\n');
    }
    out.push_str(&pad);
    out.push('}');
}

fn render_atomic_struct(
    out: &mut String,
    fields: &[(String, PropertyValue)],
    r: &RefResolver,
    pak: &UPKPak,
    depth: usize,
) {
    if fields.is_empty() {
        out.push_str("{ }");
        return;
    }
    out.push_str("{ ");
    for (i, (k, v)) in fields.iter().enumerate() {
        if i > 0 {
            out.push_str(", ");
        }
        let _ = write!(out, "{k} = ");
        render_value(out, v, r, pak, depth);
    }
    out.push_str(" }");
}

fn render_native(out: &mut String, payload: &NativePayload, class_name: &str, depth: usize) {
    let pad = INDENT.repeat(depth);
    let pad_in = INDENT.repeat(depth + 1);
    let _ = write!(out, "{pad}@native({}) {{\n", payload.variant_label());

    match payload {
        NativePayload::Empty { tail } => {
            if !tail.is_empty() {
                let _ = writeln!(
                    out,
                    "{pad_in}unexpected_tail = @bytes({} bytes)",
                    tail.len()
                );
            }
        }
        NativePayload::Raw { bytes } => {
            let head: String = bytes
                .iter()
                .take(64)
                .map(|b| format!("{b:02x}"))
                .collect::<Vec<_>>()
                .join(" ");
            let ellipsis = if bytes.len() > 64 { " …" } else { "" };
            let _ = writeln!(
                out,
                "{pad_in}bytes = @bytes({} bytes)  // no NativeSerializer for class '{class_name}'\n{pad_in}// head: {head}{ellipsis}",
                bytes.len()
            );
        }
        NativePayload::Texture2D(p) => render_texture2d(out, p, depth + 1),
        NativePayload::SwfMovie(p) => {
            let _ = writeln!(out, "{pad_in}raw_data_bytes = {}", p.raw_data.len());
        }
        NativePayload::SoundNodeWave(p) => render_sound(out, p, depth + 1),
        NativePayload::NativeProps { fields } => {
            for p in fields {
                let _ = writeln!(out, "{pad_in}{} = …", p.name);
            }
        }
    }

    let _ = writeln!(out, "{pad}}}");
}

fn render_sound(out: &mut String, p: &crate::native::SoundNodeWavePayload, depth: usize) {
    let pad = INDENT.repeat(depth);

    let _ = writeln!(
        out,
        "{pad}info = {{ channels = {}, sample_rate = {}, duration = {:.3}s }}",
        p.num_channels
            .map(|v| v.to_string())
            .unwrap_or_else(|| "?".into()),
        p.sample_rate
            .map(|v| v.to_string())
            .unwrap_or_else(|| "?".into()),
        p.duration.unwrap_or(0.0),
    );

    let blocks = [
        ("raw_data", &p.raw_data),
        ("compressed_pc", &p.compressed_pc),
        ("compressed_xbox360", &p.compressed_xbox360),
        ("compressed_ps3", &p.compressed_ps3),
    ];

    for (name, b) in blocks {
        if b.is_empty() {
            let _ = writeln!(out, "{pad}{name} = empty");
        } else {
            let sniff = crate::native::soundnodewave::AudioSniff::of(&b.data);
            let where_ = if b.is_external() {
                format!("tfc-like(offset={})", b.offset_in_file)
            } else {
                "inline".to_string()
            };
            let _ = writeln!(
                out,
                "{pad}{name} = {{ flags = 0x{:08x}, elements = {}, bytes = {}, source = {where_}, sniff = {} }}",
                b.flags,
                b.element_count,
                b.size_on_disk,
                sniff.label(),
            );
        }
    }

    if !p.channel_offsets.is_empty() || !p.channel_sizes.is_empty() {
        let _ = writeln!(out, "{pad}channels = [");
        let n = p.channel_offsets.len().max(p.channel_sizes.len());
        for i in 0..n {
            let off = p.channel_offsets.get(i).copied().unwrap_or(0);
            let sz = p.channel_sizes.get(i).copied().unwrap_or(0);
            let _ = writeln!(out, "{pad}    {{ offset = {off}, size = {sz} }}");
        }
        let _ = writeln!(out, "{pad}]");
    }

    if !p.trailing_raw.is_empty() {
        let _ = writeln!(
            out,
            "{pad}trailing = @bytes({} bytes)",
            p.trailing_raw.len()
        );
    }
}

fn render_texture2d(out: &mut String, p: &crate::native::Texture2DPayload, depth: usize) {
    let pad = INDENT.repeat(depth);
    let pad_in = INDENT.repeat(depth + 1);

    let _ = writeln!(out, "{pad}mips = [");
    for m in &p.mips {
        render_mip(out, m, depth + 1);
    }
    let _ = writeln!(out, "{pad}]");

    let g = p.tfc_guid;
    let _ = writeln!(
        out,
        "{pad}tfc_guid = 0x{:08x}_{:08x}_{:08x}_{:08x}",
        g[0] as u32, g[1] as u32, g[2] as u32, g[3] as u32
    );

    if !p.cached_pvrtc_mips.is_empty() {
        let _ = writeln!(out, "{pad}cached_pvrtc_mips = [");
        for m in &p.cached_pvrtc_mips {
            render_mip(out, m, depth + 1);
        }
        let _ = writeln!(out, "{pad}]");
    }

    if !p.trailing_raw.is_empty() {
        let _ = writeln!(
            out,
            "{pad}trailing = @bytes({} bytes)",
            p.trailing_raw.len()
        );
    }

    if let Some(f) = &p.format_label {
        let _ = writeln!(out, "{pad}// format_label captured from tagged props: {f}");
    }
    if let Some(t) = &p.tfc_name {
        let _ = writeln!(out, "{pad}// tfc_name captured from tagged props: '{t}'");
    }
    let _ = pad_in;
}

fn render_mip(out: &mut String, m: &Mip, depth: usize) {
    let pad = INDENT.repeat(depth);
    let source = match &m.source {
        MipSource::Inline => "inline".to_string(),
        MipSource::Tfc { stem_lc } => {
            if stem_lc.is_empty() {
                "tfc(?)".into()
            } else {
                format!("tfc('{stem_lc}')")
            }
        }
        MipSource::Missing => "missing".into(),
    };
    let _ = writeln!(
        out,
        "{pad}{{ size = {w}x{h}, source = {source}, bytes = {bytes}, flags = 0x{flags:08x} }}",
        w = m.size_x,
        h = m.size_y,
        bytes = m.size_on_disk,
        flags = m.flags,
    );
}
