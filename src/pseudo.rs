use std::{
    collections::HashMap,
    fmt::Write as FmtWrite,
    io::{Result, Write},
    path::{Path, PathBuf},
};

use crate::{
    native::{Mip, MipSource, NativePayload},
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

    if !matches!(input.payload, NativePayload::Empty { .. }) {
        out.push('\n');
        render_native(&mut out, input.payload, 1);
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

fn render_native(out: &mut String, payload: &NativePayload, depth: usize) {
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
            let _ = writeln!(
                out,
                "{pad_in}bytes = @bytes({} bytes)  // class has no registered serializer",
                bytes.len()
            );
        }
        NativePayload::Texture2D(p) => render_texture2d(out, p, depth + 1),
        NativePayload::SwfMovie(p) => {
            let _ = writeln!(out, "{pad_in}raw_data_bytes = {}", p.raw_data.len());
        }
    }

    let _ = writeln!(out, "{pad}}}");
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
