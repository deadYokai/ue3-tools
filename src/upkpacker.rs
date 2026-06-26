use crate::native::{NativeInjectCtx, NativeRegistry};
use crate::pseudo_parse::{self, PseudoFile, PseudoValue};
use crate::schemadb::{LazyPackage, ResolvedRef, SchemaDb, open_package_at};
use crate::upkprops::{Property, PropertyValue, read_native_props};
use crate::upkreader::{FName, UPKPak, get_obj_props_with_db};
use crate::versions::VER_NETINDEX_STORED_AS_INT;

use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};
use std::collections::HashMap;
use std::io::{Cursor, Error, ErrorKind, Result};
use std::path::{Path, PathBuf};
use std::rc::Rc;

pub struct PackOptions<'a> {
    pub extracted_dir: &'a Path,
    pub game_root: Option<&'a Path>,
    pub out_dir: Option<&'a Path>,
    pub verbose: bool,
}

pub fn pack_mod(opts: &PackOptions) -> Result<()> {
    let uo_files = find_uo_files(opts.extracted_dir)?;
    if uo_files.is_empty() {
        eprintln!(
            "pack-mod: no .uo files found under {}",
            opts.extracted_dir.display()
        );
        return Ok(());
    }

    let mut by_pkg: HashMap<String, Vec<(PathBuf, PseudoFile)>> = HashMap::new();
    let mut skipped_defs = 0usize;
    for path in &uo_files {
        let text = std::fs::read_to_string(path)?;
        let parsed = match pseudo_parse::parse(&text) {
            Ok(p) => p,
            Err(e) => {
                eprintln!("  SKIP {} — {e}", path.display());
                continue;
            }
        };
        if parsed.is_definition {
            skipped_defs += 1;
            continue;
        }
        let stem = match &parsed.pkg_stem {
            Some(s) => s.to_lowercase(),
            None => {
                eprintln!("  SKIP {} — header missing 'pkg='", path.display());
                continue;
            }
        };
        by_pkg.entry(stem).or_default().push((path.clone(), parsed));
    }

    let out_dir = match opts.out_dir {
        Some(dir) => dir.to_path_buf(),
        None => overrides_dir(opts.extracted_dir),
    };
    std::fs::create_dir_all(&out_dir)?;

    let mut written = 0usize;
    let mut failed = 0usize;
    for (stem, targets) in &by_pkg {
        let lp = match load_package(stem, opts) {
            Ok(lp) => lp,
            Err(e) => {
                eprintln!(
                    "  package '{stem}' — {e}; skipping {} object(s)",
                    targets.len()
                );
                failed += targets.len();
                continue;
            }
        };
        let db = build_db(opts, &lp)?;

        let pkg_name = lp
            .path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or(stem.as_str())
            .to_string();
        let pkg_dir = out_dir.join(&pkg_name);
        std::fs::create_dir_all(&pkg_dir)?;
        let mut names = lp.pak.name_table.clone();

        let mut pkg_ok = 0usize;

        for (src_path, uo) in targets {
            match pack_one(&lp, db.as_ref(), uo, src_path, &pkg_dir, &mut names) {
                Ok(key) => {
                    written += 1;
                    pkg_ok += 1;
                    if opts.verbose {
                        println!("  OK   {key}  <-  {}", src_path.display());
                    }
                }
                Err(e) => {
                    failed += 1;
                    eprintln!("  FAIL {}  —  {e}", src_path.display());
                }
            }
        }

        if pkg_ok > 0 {
            let map_path = pkg_dir.join(format!("{pkg_name}.namemap"));
            std::fs::write(&map_path, names.join("\n"))?;
        }
    }

    println!(
        "pack-mod: {written} override(s) written to {}  ({failed} failed, {skipped_defs} definition(s) skipped)",
        out_dir.display()
    );
    Ok(())
}

fn pack_one(
    lp: &LazyPackage,
    db: Option<&SchemaDb>,
    uo: &PseudoFile,
    uo_path: &Path,
    pkg_dir: &Path,
    names: &mut Vec<String>,
) -> Result<String> {
    let pak = &lp.pak;
    let p_ver = lp.header.p_ver;

    let export_idx = resolve_export_index(pak, uo)?;
    let class_index = pak
        .export_table
        .get((export_idx - 1) as usize)
        .map(|e| e.class_index)
        .unwrap_or(0);
    let owner = if class_index > 0 {
        Some(ResolvedRef {
            stem_lc: lp.stem_lc.clone(),
            export_idx: class_index,
        })
    } else if class_index < 0 {
        db.and_then(|d| {
            d.open_package(&lp.stem_lc)
                .ok()
                .and_then(|lp2| d.resolve_index(&lp2, class_index).ok().flatten())
        })
    } else {
        None
    };
    let blob = lp.export_blob(export_idx)?.to_vec();
    let mut cur = Cursor::new(&blob);
    let net_index = if p_ver >= VER_NETINDEX_STORED_AS_INT {
        Some(cur.read_i32::<LittleEndian>()?)
    } else {
        None
    };
    let (mut props, props_end) =
        get_obj_props_with_db(&mut cur, pak, false, p_ver, db, owner.clone())?;
    let mut native_tail = blob[props_end as usize..].to_vec();

    let mut native_fields: Option<Vec<Property>> = None;
    if !uo.native_fields.is_empty() {
        native_fields = match (db, owner.as_ref()) {
            (Some(sdb), Some(class_ref)) if !native_tail.is_empty() => {
                read_native_props(&native_tail, pak, p_ver, sdb, class_ref, &props).filter(|p| {
                    let reenc = match native_props_to_bytes(p, pak, p_ver) {
                        Ok(b) => b,
                        Err(_) => return false,
                    };
                    match read_native_props(&reenc, pak, p_ver, sdb, class_ref, &props) {
                        Some(p2) => native_props_to_bytes(&p2, pak, p_ver)
                            .map(|b2| b2 == reenc)
                            .unwrap_or(false),
                        None => false,
                    }
                })
            }
            _ => None,
        };
        if native_fields.is_none() {
            eprintln!(
                "  note: native-serialized field(s) for {} are not re-encodable here; \
                 kept verbatim (edits to them ignored)",
                uo.full_path.as_deref().unwrap_or("<obj>")
            );
        }
    }

    let edits = collect_edits(uo);
    apply_edits(&mut props, &edits, pak, names)?;

    if let Some(nf) = native_fields.as_mut() {
        for (name, edit) in &uo.native_fields {
            match nf.iter_mut().find(|p| &p.name == name) {
                Some(prop) => {
                    overlay_value(&mut prop.value, edit, pak, names).map_err(|e| ctx(name, e))?
                }
                None => eprintln!(
                    "  note: native field '{name}' not present in parsed native data; skipped"
                ),
            }
        }
    }

    inject_sidecars(
        &mut props,
        &mut native_tail,
        pak,
        uo,
        uo_path,
        db,
        owner.as_ref(),
        class_index,
        p_ver,
    )?;

    ensure_tag_names(&props, names);

    let working = UPKPak {
        name_table: names.clone(),
        export_table: pak.export_table.clone(),
        import_table: pak.import_table.clone(),
    };

    let mut body: Vec<u8> = Vec::with_capacity(blob.len());
    {
        let mut w = Cursor::new(&mut body);
        if let Some(n) = net_index {
            w.write_i32::<LittleEndian>(n)?;
        }
        for p in &props {
            p.write(&mut w, &working, p_ver)?;
        }
    }

    match &native_fields {
        Some(nf) => body.extend_from_slice(&native_props_to_bytes(nf, &working, p_ver)?),
        None => body.extend_from_slice(&native_tail),
    }

    let key = export_path_dotted(pak, export_idx);
    let bin_path = pkg_dir.join(format!("{key}.bin"));
    std::fs::write(&bin_path, &body)?;
    Ok(key)
}

fn inject_sidecars(
    props: &mut Vec<Property>,
    native_tail: &mut Vec<u8>,
    pak: &UPKPak,
    uo: &PseudoFile,
    uo_path: &Path,
    db: Option<&SchemaDb>,
    owner: Option<&ResolvedRef>,
    class_index: i32,
    p_ver: i16,
) -> Result<()> {
    if uo.sidecars.is_empty() {
        return Ok(());
    }
    let class_name = pak.get_class_name(class_index);
    let registry = NativeRegistry::standard();
    let ser = match registry.for_class(db, owner, &class_name) {
        Some(s) => s,
        None => {
            eprintln!(
                "  note: {} sidecar(s) listed but no native serializer for class '{}'; ignored",
                uo.sidecars.len(),
                class_name
            );

            return Ok(());
        }
    };

    let externalized_prop = uo.fields.iter().find_map(|(name, v)| match v {
        PseudoValue::Opaque(s) if s.trim_start().starts_with("@sidecar") => Some(name.clone()),
        _ => None,
    });

    let dir = uo_path.parent().unwrap_or_else(|| Path::new("."));

    let mut ictx = NativeInjectCtx {
        props,
        native_tail,
        sidecar_dir: dir,
        sidecars: &uo.sidecars,
        externalized_prop,
        ver: p_ver,
        pak,
    };
    ser.inject_external(&mut ictx)?;
    Ok(())
}

fn collect_edits(uo: &PseudoFile) -> Vec<(String, PseudoValue)> {
    uo.fields
        .iter()
        .filter(|(name, val)| name != "@native" && !matches!(val, PseudoValue::Opaque(_)))
        .cloned()
        .collect()
}

fn apply_edits(
    props: &mut [Property],
    edits: &[(String, PseudoValue)],
    pak: &UPKPak,
    names: &mut Vec<String>,
) -> Result<()> {
    for (name, edit) in edits {
        match props.iter_mut().find(|p| &p.name == name) {
            Some(prop) => {
                overlay_value(&mut prop.value, edit, pak, names).map_err(|e| ctx(name, e))?
            }
            None => {
                return Err(Error::new(
                    ErrorKind::InvalidInput,
                    format!(
                        "property '{name}' is not in the original export; \
                         adding new properties needs schema support (v1 edits \
                         existing properties only)"
                    ),
                ));
            }
        }
    }
    Ok(())
}

fn overlay_value(
    orig: &mut PropertyValue,
    edit: &PseudoValue,
    pak: &UPKPak,
    names: &mut Vec<String>,
) -> Result<()> {
    use PropertyValue as PV;
    match (orig, edit) {
        (PV::Int(slot), PseudoValue::Num(s)) => *slot = parse_i32(s)?,
        (PV::Byte(slot), PseudoValue::Num(s)) => *slot = parse_u8(s)?,
        (PV::Float(slot), PseudoValue::Num(s)) => *slot = parse_f32(s)?,
        (PV::Bool(slot), PseudoValue::Bool(b)) => *slot = *b,
        (PV::String(slot), PseudoValue::Str(s)) => *slot = s.clone(),

        (PV::Name(slot), PseudoValue::Name(s)) => *slot = intern_fname(s, names),

        (PV::EnumLabel(slot), PseudoValue::Enum(s)) => {
            let val = s.rsplit("::").next().unwrap_or(s);
            ensure_name(val, names);
            *slot = s.clone();
        }

        (PV::Object(idx), PseudoValue::Ref(label)) => {
            if let Some(new_idx) = resolve_ref(pak, label) {
                *idx = new_idx;
            } else if label_matches_index(pak, *idx, label) {
            } else {
                return Err(Error::new(
                    ErrorKind::InvalidInput,
                    format!(
                        "could not resolve object reference '&{label}' in package \
                         (label form is lossy; use a fuller path)"
                    ),
                ));
            }
        }
        (PV::Object(idx), PseudoValue::Null) => *idx = 0,

        (PV::Array(items), PseudoValue::Array(eitems)) => {
            overlay_array(items, eitems, pak, names)?;
        }

        (PV::Struct(fields), PseudoValue::Object(efields)) => {
            for (fname, ev) in efields {
                match fields.iter_mut().find(|p| &p.name == fname) {
                    Some(fp) => {
                        overlay_value(&mut fp.value, ev, pak, names).map_err(|e| ctx(fname, e))?
                    }
                    None => return Err(unknown_field(fname)),
                }
            }
        }

        (PV::AtomicStruct(fields), PseudoValue::Object(efields)) => {
            for (fname, ev) in efields {
                match fields.iter_mut().find(|(k, _)| k == fname) {
                    Some((_, fv)) => {
                        overlay_value(fv, ev, pak, names).map_err(|e| ctx(fname, e))?
                    }
                    None => return Err(unknown_field(fname)),
                }
            }
        }

        (_, PseudoValue::Opaque(_)) => {}

        (orig_slot, e) => {
            return Err(Error::new(
                ErrorKind::InvalidInput,
                format!(
                    "type mismatch: original is {} but edit is {}",
                    variant_name(orig_slot),
                    uo_variant_name(e)
                ),
            ));
        }
    }
    Ok(())
}

fn overlay_array(
    items: &mut Vec<PropertyValue>,
    eitems: &[PseudoValue],
    pak: &UPKPak,
    names: &mut Vec<String>,
) -> Result<()> {
    if eitems.len() < items.len() {
        items.truncate(eitems.len());
    }

    for (i, ev) in eitems.iter().enumerate() {
        if i < items.len() {
            overlay_value(&mut items[i], ev, pak, names).map_err(|e| ctx(&format!("[{i}]"), e))?;
        } else {
            let template = items.first().cloned().ok_or_else(|| {
                Error::new(
                    ErrorKind::InvalidInput,
                    "cannot grow an empty array (no template element to infer \
                     element type from)"
                        .to_string(),
                )
            })?;
            let mut el = template;
            overlay_value(&mut el, ev, pak, names).map_err(|e| ctx(&format!("[{i}]"), e))?;
            items.push(el);
        }
    }
    Ok(())
}

fn intern_fname(s: &str, names: &mut Vec<String>) -> FName {
    let (base, instance) = split_instance(s);
    let idx = ensure_name(&base, names);
    FName {
        name_index: idx,
        name_instance: instance,
    }
}

fn split_instance(s: &str) -> (String, i32) {
    if let Some(pos) = s.rfind('_') {
        let (head, tail) = s.split_at(pos);
        let digits = &tail[1..];
        if !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()) {
            if let Ok(n) = digits.parse::<i32>() {
                return (head.to_string(), n + 1);
            }
        }
    }
    (s.to_string(), 0)
}

fn ensure_name(name: &str, names: &mut Vec<String>) -> i32 {
    if let Some(i) = names.iter().position(|n| n == name) {
        return i as i32;
    }
    names.push(name.to_string());
    (names.len() - 1) as i32
}

fn ensure_tag_names(props: &[Property], names: &mut Vec<String>) {
    ensure_name("None", names);
    for p in props {
        ensure_name(&p.name, names);
        if p.name == "None" {
            continue;
        }
        ensure_name(&p.prop_type, names);
        if let Some(sn) = &p.struct_name {
            ensure_name(sn, names);
        }
        if let Some(en) = &p.enum_name {
            ensure_name(en, names);
        }
        ensure_value_names(&p.value, names);
    }
}

fn ensure_value_names(v: &PropertyValue, names: &mut Vec<String>) {
    match v {
        PropertyValue::EnumLabel(s) => {
            let val = s.rsplit("::").next().unwrap_or(s);
            ensure_name(val, names);
        }
        PropertyValue::Array(items) => {
            for it in items {
                ensure_value_names(it, names);
            }
        }
        PropertyValue::Struct(fields) => ensure_tag_names(fields, names),
        PropertyValue::AtomicStruct(fields) => {
            for (_, fv) in fields {
                ensure_value_names(fv, names);
            }
        }
        _ => {}
    }
}

fn resolve_ref(pak: &UPKPak, label: &str) -> Option<i32> {
    let label = label.trim();
    if label.is_empty() || label == "None" {
        return Some(0);
    }
    if label.starts_with('<') {
        return None;
    }

    if let Some(rest) = label.strip_prefix("extern:") {
        let leaf = rest.rsplit("::").next().unwrap_or(rest);
        for (i, imp) in pak.import_table.iter().enumerate() {
            if pak.fname_to_string(&imp.object_name) == leaf {
                return Some(-(i as i32) - 1);
            }
        }
        return None;
    }

    for i in 0..pak.export_table.len() as i32 {
        let idx = i + 1;
        if export_path_dotted(pak, idx) == label {
            return Some(idx);
        }
    }
    let leaf = label.rsplit('.').next().unwrap_or(label);
    let mut hit = None;
    for (i, exp) in pak.export_table.iter().enumerate() {
        if pak.fname_to_string(&exp.object_name) == leaf {
            if hit.is_some() {
                return None;
            }
            hit = Some((i as i32) + 1);
        }
    }
    hit
}

fn label_matches_index(pak: &UPKPak, idx: i32, label: &str) -> bool {
    if idx == 0 {
        return label == "None";
    }
    if idx > 0 {
        let leaf = pak
            .export_table
            .get((idx - 1) as usize)
            .map(|e| pak.fname_to_string(&e.object_name));
        if let Some(leaf) = leaf {
            return label == leaf || export_path_dotted(pak, idx) == label;
        }
    }
    false
}

fn export_path_dotted(pak: &UPKPak, export_index: i32) -> String {
    let mut parts = Vec::new();
    let mut cur = export_index;
    let mut guard = 0;
    while cur > 0 && guard < 32 {
        guard += 1;
        let exp = match pak.export_table.get((cur - 1) as usize) {
            Some(e) => e,
            None => break,
        };
        parts.push(pak.fname_to_string(&exp.object_name));
        cur = exp.outer_index;
    }
    parts.reverse();
    parts.join(".")
}

fn resolve_export_index(pak: &UPKPak, uo: &PseudoFile) -> Result<i32> {
    if let Some(full) = &uo.full_path {
        for i in 0..pak.export_table.len() as i32 {
            let idx = i + 1;
            if pak.get_export_full_name(idx) == *full {
                return Ok(idx);
            }
        }
    }
    if let Some(n) = uo.export_index {
        if n >= 1 && (n as usize) <= pak.export_table.len() {
            return Ok(n);
        }
    }
    Err(Error::new(
        ErrorKind::NotFound,
        format!(
            "could not locate export for '{}' (#{:?}) in package",
            uo.full_path.as_deref().unwrap_or("<no path>"),
            uo.export_index
        ),
    ))
}

fn load_package(stem: &str, opts: &PackOptions) -> Result<LazyPackage> {
    let path = find_package_file(stem, opts).ok_or_else(|| {
        Error::new(
            ErrorKind::NotFound,
            format!(
                "package '{stem}.upk' not found under --game-root or {}",
                opts.extracted_dir.display()
            ),
        )
    })?;
    open_package_at(&path, stem)
}

fn build_db(opts: &PackOptions, lp: &LazyPackage) -> Result<Option<SchemaDb>> {
    let Some(root) = opts.game_root else {
        return Ok(None);
    };
    let db = SchemaDb::new(root)?.with_verbose(opts.verbose);

    db.inject_package(Rc::new(LazyPackage {
        stem_lc: lp.stem_lc.clone(),
        path: lp.path.clone(),
        bytes: lp.bytes.clone(),
        header: lp.header.clone(),
        pak: lp.pak.clone(),
    }));
    Ok(Some(db))
}

fn find_package_file(stem: &str, opts: &PackOptions) -> Option<PathBuf> {
    let want = stem.to_lowercase();
    let mut dirs: Vec<PathBuf> = Vec::new();

    if let Some(gr) = opts.game_root {
        dirs.push(gr.to_path_buf());
    }
    dirs.push(opts.extracted_dir.to_path_buf());
    for dir in dirs {
        let entries = match std::fs::read_dir(&dir) {
            Ok(e) => e,
            Err(_) => continue,
        };
        for ent in entries.flatten() {
            let p = ent.path();
            if !p.is_file() {
                continue;
            }
            let stem_lc = p
                .file_stem()
                .and_then(|s| s.to_str())
                .map(|s| s.to_lowercase());
            let ext_lc = p
                .extension()
                .and_then(|s| s.to_str())
                .map(|s| s.to_lowercase());
            if stem_lc.as_deref() == Some(want.as_str())
                && matches!(ext_lc.as_deref(), Some("upk") | Some("package") | None)
            {
                return Some(p);
            }
        }
    }
    None
}

fn overrides_dir(extracted_dir: &Path) -> PathBuf {
    for name in ["overrides", "Overrides"] {
        let c = extracted_dir.join(name);
        if c.is_dir() {
            return c;
        }
    }
    extracted_dir.join("overrides")
}

fn find_uo_files(root: &Path) -> Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    walk(root, &mut out)?;
    out.sort();
    Ok(out)
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    if !dir.is_dir() {
        return Ok(());
    }
    for ent in std::fs::read_dir(dir)?.flatten() {
        let p = ent.path();
        if p.is_dir() {
            walk(&p, out)?;
        } else if p.extension().and_then(|s| s.to_str()) == Some("uo") {
            out.push(p);
        }
    }
    Ok(())
}

#[allow(dead_code)]
fn encode_native_payload(_class_name: &str, _uo: &PseudoFile) -> Option<Vec<u8>> {
    None
}

fn parse_i32(s: &str) -> Result<i32> {
    let v = if let Some(h) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        i64::from_str_radix(h, 16)
    } else {
        s.parse::<i64>()
    }
    .map_err(|_| num_err(s, "i32"))?;
    Ok(v as i32)
}

fn parse_u8(s: &str) -> Result<u8> {
    let v = if let Some(h) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        i64::from_str_radix(h, 16)
    } else {
        s.parse::<i64>()
    }
    .map_err(|_| num_err(s, "u8"))?;
    if !(0..=255).contains(&v) {
        return Err(num_err(s, "u8 (out of range)"));
    }
    Ok(v as u8)
}

fn parse_f32(s: &str) -> Result<f32> {
    s.parse::<f32>().map_err(|_| num_err(s, "f32"))
}

fn num_err(s: &str, ty: &str) -> Error {
    Error::new(
        ErrorKind::InvalidInput,
        format!("'{s}' is not a valid {ty}"),
    )
}

fn ctx(field: &str, e: Error) -> Error {
    Error::new(e.kind(), format!("{field}: {e}"))
}

fn unknown_field(name: &str) -> Error {
    Error::new(
        ErrorKind::InvalidInput,
        format!("struct has no field '{name}' in the original export"),
    )
}

fn variant_name(v: &PropertyValue) -> &'static str {
    use PropertyValue as P;
    match v {
        P::None => "None",
        P::Byte(_) => "Byte",
        P::Int(_) => "Int",
        P::Bool(_) => "Bool",
        P::Float(_) => "Float",
        P::Object(_) => "Object",
        P::ObjectRef(_) => "ObjectRef",
        P::Name(_) => "Name",
        P::EnumLabel(_) => "EnumLabel",
        P::String(_) => "String",
        P::Array(_) => "Array",
        P::Struct(_) => "Struct",
        P::AtomicStruct(_) => "AtomicStruct",
        P::Raw(_) => "Raw",
    }
}

fn uo_variant_name(v: &PseudoValue) -> &'static str {
    match v {
        PseudoValue::Null => "None",
        PseudoValue::Num(_) => "number",
        PseudoValue::Bool(_) => "bool",
        PseudoValue::Str(_) => "string",
        PseudoValue::Name(_) => "name",
        PseudoValue::Enum(_) => "enum",
        PseudoValue::Ref(_) => "object-ref",
        PseudoValue::Array(_) => "array",
        PseudoValue::Object(_) => "struct",
        PseudoValue::Opaque(_) => "opaque",
    }
}

fn native_props_to_bytes(fields: &[Property], pak: &UPKPak, ver: i16) -> Result<Vec<u8>> {
    let mut buf = Vec::new();
    let mut c = Cursor::new(&mut buf);
    for p in fields {
        p.value.write_all(&mut c, pak, ver)?;
    }
    Ok(buf)
}
