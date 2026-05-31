use std::{
    cell::RefCell,
    collections::{HashMap, HashSet, VecDeque},
    fs::File,
    io::{BufReader, Cursor, Error, ErrorKind, Read, Result, Seek, SeekFrom},
    path::{Path, PathBuf},
    rc::Rc,
};

use byteorder::{LittleEndian, ReadBytesExt};

use crate::{
    schema::{PropertyKind, SchemaEntry, SchemaParseCtx, parse_export_schema},
    upkdecompress::{CompressionMethod, upk_decompress},
    upkreader::{FName, PackageFlags, UPKPak, UpkHeader},
    versions::VER_BYTEPROP_SERIALIZE_ENUM,
};

#[derive(Debug, Clone, Eq, PartialEq, Hash)]
pub struct ResolvedRef {
    pub stem_lc: String,
    pub export_idx: i32,
}

impl ResolvedRef {
    pub fn display(&self) -> String {
        format!("{}::#{}", self.stem_lc, self.export_idx)
    }
}

pub struct LazyPackage {
    pub stem_lc: String,
    pub path: PathBuf,
    pub bytes: Vec<u8>,
    pub header: UpkHeader,
    pub pak: UPKPak,
}

impl LazyPackage {
    pub fn export_blob(&self, i: i32) -> Result<&[u8]> {
        let idx = (i - 1) as usize;
        let exp = self.pak.export_table.get(idx).ok_or_else(|| {
            Error::new(
                ErrorKind::InvalidInput,
                format!("export #{i} out of range in {}", self.stem_lc),
            )
        })?;
        let s = exp.serial_offset as usize;
        let e = s.saturating_add(exp.serial_size as usize);
        if e > self.bytes.len() {
            return Err(Error::new(
                ErrorKind::UnexpectedEof,
                format!(
                    "export #{i} serial range [{s}, {e}) past EOF in {}",
                    self.stem_lc
                ),
            ));
        }
        Ok(&self.bytes[s..e])
    }

    pub fn export_class_name(&self, i: i32) -> String {
        let idx = (i - 1) as usize;
        let exp = match self.pak.export_table.get(idx) {
            Some(e) => e,
            None => return String::new(),
        };
        if exp.class_index > 0 {
            let cidx = (exp.class_index - 1) as usize;
            self.pak
                .export_table
                .get(cidx)
                .map(|c| self.pak.fname_to_string(&c.object_name))
                .unwrap_or_default()
        } else if exp.class_index < 0 {
            let cidx = (-exp.class_index - 1) as usize;
            self.pak
                .import_table
                .get(cidx)
                .map(|c| self.pak.fname_to_string(&c.object_name))
                .unwrap_or_default()
        } else {
            "Class".into()
        }
    }

    pub fn export_full_name(&self, i: i32) -> String {
        self.pak.get_export_full_name(i)
    }

    pub fn schema_ctx(&self) -> SchemaParseCtx {
        SchemaParseCtx {
            p_ver: self.header.p_ver,
            strip_editor_only: self.header.strip_editor_only(),
        }
    }

    pub fn is_cooked(&self) -> bool {
        self.header.pak_flags & PackageFlags::Cooked.bits() != 0
    }
}

pub struct SchemaDb {
    pub game_root: PathBuf,
    pub stem_index: HashMap<String, PathBuf>,
    pub tfc_index: HashMap<String, PathBuf>,

    packages: RefCell<HashMap<String, Rc<LazyPackage>>>,
    entries: RefCell<HashMap<(String, i32), Rc<SchemaEntry>>>,
    visiting: RefCell<HashSet<(String, i32)>>,

    pub misses: RefCell<Vec<String>>,
    pub verbose: bool,
}

impl SchemaDb {
    pub fn new(game_root: &Path) -> Result<Self> {
        let mut stem_index = HashMap::new();
        let mut tfc_index = HashMap::new();

        if !game_root.as_os_str().is_empty() {
            walk_index(game_root, &mut stem_index, &mut tfc_index)?;
        }

        Ok(Self {
            game_root: game_root.to_path_buf(),
            stem_index,
            tfc_index,
            packages: RefCell::new(HashMap::new()),
            entries: RefCell::new(HashMap::new()),
            visiting: RefCell::new(HashSet::new()),
            misses: RefCell::new(Vec::new()),
            verbose: false,
        })
    }

    pub fn with_verbose(mut self, v: bool) -> Self {
        self.verbose = v;
        self
    }

    pub fn inject_package(&self, pkg: Rc<LazyPackage>) {
        self.packages.borrow_mut().insert(pkg.stem_lc.clone(), pkg);
    }

    pub fn open_package(&self, stem: &str) -> Result<Rc<LazyPackage>> {
        let key = stem.to_lowercase();
        if let Some(p) = self.packages.borrow().get(&key) {
            return Ok(p.clone());
        }
        let path = self.stem_index.get(&key).cloned().ok_or_else(|| {
            Error::new(
                ErrorKind::NotFound,
                format!("package '{stem}' not in --game-root"),
            )
        })?;
        let lp = Rc::new(open_package_at(&path, &key)?);
        self.packages.borrow_mut().insert(key, lp.clone());
        Ok(lp)
    }

    fn note_miss(&self, msg: String) {
        if self.verbose {
            eprintln!("  schemadb: {msg}");
        }
        self.misses.borrow_mut().push(msg);
    }

    pub fn resolve_index(&self, pkg: &LazyPackage, idx: i32) -> Result<Option<ResolvedRef>> {
        if idx == 0 {
            return Ok(None);
        }
        if idx > 0 {
            return Ok(Some(ResolvedRef {
                stem_lc: pkg.stem_lc.clone(),
                export_idx: idx,
            }));
        }
        self.resolve_import(pkg, idx)
    }

    fn resolve_import(&self, pkg: &LazyPackage, idx: i32) -> Result<Option<ResolvedRef>> {
        let imp_idx = (-idx - 1) as usize;
        let imp = match pkg.pak.import_table.get(imp_idx) {
            Some(v) => v,
            None => {
                self.note_miss(format!(
                    "import #{} out of range in {}",
                    -idx - 1,
                    pkg.stem_lc
                ));
                return Ok(None);
            }
        };

        let top_pkg = top_package_name(&pkg.pak, idx);
        let top_pkg = match top_pkg {
            Some(s) => s,
            None => {
                self.note_miss(format!(
                    "could not climb to top package for import #{} in {}",
                    -idx - 1,
                    pkg.stem_lc
                ));
                return Ok(None);
            }
        };

        let other = match self.open_package(&top_pkg) {
            Ok(p) => p,
            Err(e) => {
                self.note_miss(format!(
                    "open '{}' failed (importing {}): {}",
                    top_pkg,
                    pkg.pak.get_import_full_name(idx),
                    e
                ));
                return Ok(None);
            }
        };

        let want_name = pkg.pak.fname_to_string(&imp.object_name);
        let want_class = pkg.pak.fname_to_string(&imp.class_name);
        let want_class_pkg = pkg.pak.fname_to_string(&imp.class_package);
        let import_chain = collect_import_chain(&pkg.pak, idx);

        if let Some(eidx) = find_export_matching(
            &other,
            &want_name,
            &want_class,
            &want_class_pkg,
            &import_chain,
        ) {
            return Ok(Some(ResolvedRef {
                stem_lc: other.stem_lc.clone(),
                export_idx: eidx,
            }));
        }

        if let Some(redir_eidx) = find_export_matching(
            &other,
            &want_name,
            "ObjectRedirector",
            "Core",
            &import_chain,
        ) {
            if let Some(dest_idx) = read_redirector_destination(&other, redir_eidx)? {
                if self.verbose {
                    eprintln!(
                        "  schemadb: redirector {} → #{} in {}",
                        pkg.pak.get_import_full_name(idx),
                        dest_idx,
                        other.stem_lc
                    );
                }
                return Ok(Some(ResolvedRef {
                    stem_lc: other.stem_lc.clone(),
                    export_idx: dest_idx,
                }));
            }
        }

        self.note_miss(format!(
            "no export {} (class {}.{}) in {}",
            want_name, want_class_pkg, want_class, other.stem_lc
        ));
        Ok(None)
    }

    pub fn resolve_full_path(
        &self,
        starting_pkg_stem: &str,
        full_path: &str,
    ) -> Result<Option<ResolvedRef>> {
        let _start = self.open_package(starting_pkg_stem)?;
        let (class_name, dotted) = split_full_name(full_path);

        let mut parts = dotted.split(['.', ':']);
        let pkg_seg = parts.next().unwrap_or("");
        let chain: Vec<&str> = parts.collect();
        if chain.is_empty() {
            return Ok(None);
        }
        let target = match self.open_package(pkg_seg) {
            Ok(t) => t,
            Err(_) => {
                self.note_miss(format!("'{pkg_seg}' not in game-root"));
                return Ok(None);
            }
        };

        let mut outer_export: Option<i32> = None;
        for (i, seg) in chain.iter().enumerate() {
            let want_class = if i + 1 == chain.len() { class_name } else { "" };
            outer_export =
                find_export_by_name_and_outer(&target, seg, want_class, outer_export.unwrap_or(0));
            if outer_export.is_none() {
                self.note_miss(format!(
                    "{full_path}: segment '{seg}' not found in {}",
                    target.stem_lc
                ));
                return Ok(None);
            }
        }
        Ok(outer_export.map(|e| ResolvedRef {
            stem_lc: target.stem_lc.clone(),
            export_idx: e,
        }))
    }

    pub fn entry(&self, r: &ResolvedRef) -> Result<Rc<SchemaEntry>> {
        let key = (r.stem_lc.clone(), r.export_idx);
        if let Some(e) = self.entries.borrow().get(&key) {
            return Ok(e.clone());
        }
        if self.visiting.borrow().contains(&key) {
            return Err(Error::new(
                ErrorKind::WouldBlock,
                format!("schema cycle on {}", r.display()),
            ));
        }
        self.visiting.borrow_mut().insert(key.clone());

        let pkg = self.open_package(&r.stem_lc)?;
        let blob = pkg.export_blob(r.export_idx)?.to_vec();
        let class_name = pkg.export_class_name(r.export_idx);
        let ctx = pkg.schema_ctx();
        let parsed = parse_export_schema(&blob, &class_name, &pkg.pak, ctx);

        self.visiting.borrow_mut().remove(&key);

        let entry = match parsed {
            Ok(Some(e)) => Rc::new(e),
            Ok(None) => {
                return Err(Error::new(
                    ErrorKind::InvalidData,
                    format!(
                        "{}::#{} has class '{}' not handled by schema parser",
                        r.stem_lc, r.export_idx, class_name
                    ),
                ));
            }
            Err(e) => return Err(e),
        };
        self.entries.borrow_mut().insert(key, entry.clone());
        Ok(entry)
    }

    pub fn list_children(
        &self,
        r: &ResolvedRef,
    ) -> Result<Vec<(String, ResolvedRef, Rc<SchemaEntry>)>> {
        let pkg = self.open_package(&r.stem_lc)?;
        let entry = self.entry(r)?;
        let head = match &*entry {
            SchemaEntry::Class { header, .. }
            | SchemaEntry::State { header, .. }
            | SchemaEntry::ScriptStruct { header, .. }
            | SchemaEntry::Struct { header }
            | SchemaEntry::Function { header, .. } => header.children,
            _ => return Ok(Vec::new()),
        };

        let mut out = Vec::new();
        let mut cur = head;
        let mut guard = 0;
        while cur != 0 {
            guard += 1;
            if guard > 8192 {
                self.note_miss(format!(
                    "children walk runaway at {} (cur={cur})",
                    r.display()
                ));
                break;
            }
            let child_ref = match self.resolve_index(&pkg, cur)? {
                Some(rr) => rr,
                None => break,
            };
            let child_entry = match self.entry(&child_ref) {
                Ok(e) => e,
                Err(e) => {
                    self.note_miss(format!("child {}: {}", child_ref.display(), e));
                    break;
                }
            };
            let child_name = self
                .export_object_name(&child_ref)
                .unwrap_or_else(|| format!("#{}", child_ref.export_idx));
            let next_cur = child_entry.common_next();
            out.push((child_name, child_ref, child_entry));
            cur = next_cur;
        }
        Ok(out)
    }

    pub fn class_chain(&self, r: &ResolvedRef) -> Result<Vec<ResolvedRef>> {
        let mut chain = vec![r.clone()];
        let mut cur = r.clone();
        let mut guard = 0;
        loop {
            guard += 1;
            if guard > 64 {
                self.note_miss(format!("class_chain runaway at {}", r.display()));
                break;
            }
            let pkg = self.open_package(&cur.stem_lc)?;
            let entry = self.entry(&cur)?;
            let super_idx = match &*entry {
                SchemaEntry::Class { header, .. }
                | SchemaEntry::State { header, .. }
                | SchemaEntry::ScriptStruct { header, .. }
                | SchemaEntry::Struct { header }
                | SchemaEntry::Function { header, .. } => header.super_struct,
                _ => 0,
            };
            if super_idx == 0 {
                break;
            }
            match self.resolve_index(&pkg, super_idx)? {
                Some(next) => {
                    if chain.iter().any(|r| r == &next) {
                        break;
                    }
                    chain.push(next.clone());
                    cur = next;
                }
                None => break,
            }
        }
        Ok(chain)
    }

    pub fn find_property(
        &self,
        class_ref: &ResolvedRef,
        name: &str,
    ) -> Result<Option<(ResolvedRef, Rc<SchemaEntry>)>> {
        for klass in self.class_chain(class_ref)? {
            for (cname, cref, centry) in self.list_children(&klass)? {
                if cname == name && matches!(&*centry, SchemaEntry::Property(_)) {
                    return Ok(Some((cref, centry)));
                }
            }
        }
        Ok(None)
    }

    pub fn export_object_name(&self, r: &ResolvedRef) -> Option<String> {
        let pkg = self.packages.borrow().get(&r.stem_lc).cloned()?;
        let idx = (r.export_idx - 1) as usize;
        pkg.pak
            .export_table
            .get(idx)
            .map(|e| pkg.pak.fname_to_string(&e.object_name))
    }

    pub fn array_inner_for(
        &self,
        class_ref: &ResolvedRef,
        prop_name: &str,
    ) -> Result<Option<(ResolvedRef, Rc<SchemaEntry>)>> {
        let (_, entry) = match self.find_property(class_ref, prop_name)? {
            Some(p) => p,
            None => return Ok(None),
        };
        let inner_idx = match &*entry {
            SchemaEntry::Property(PropertyKind::Array { inner, .. }) => *inner,
            _ => return Ok(None),
        };
        let pkg = self.open_package(&class_ref.stem_lc)?;
        let inner_ref = match self.resolve_index(&pkg, inner_idx)? {
            Some(r) => r,
            None => return Ok(None),
        };
        let inner_entry = self.entry(&inner_ref)?;
        Ok(Some((inner_ref, inner_entry)))
    }

    pub fn struct_for(
        &self,
        class_ref: &ResolvedRef,
        prop_name: &str,
    ) -> Result<Option<(ResolvedRef, Rc<SchemaEntry>)>> {
        let (_, entry) = match self.find_property(class_ref, prop_name)? {
            Some(p) => p,
            None => return Ok(None),
        };
        let struct_idx = match &*entry {
            SchemaEntry::Property(PropertyKind::Struct { struct_obj, .. }) => *struct_obj,
            _ => return Ok(None),
        };
        let pkg = self.open_package(&class_ref.stem_lc)?;
        let struct_ref = match self.resolve_index(&pkg, struct_idx)? {
            Some(r) => r,
            None => return Ok(None),
        };
        Ok(Some((struct_ref.clone(), self.entry(&struct_ref)?)))
    }

    pub fn enum_names_for(
        &self,
        class_ref: &ResolvedRef,
        prop_name: &str,
    ) -> Result<Option<Vec<String>>> {
        let (_, entry) = match self.find_property(class_ref, prop_name)? {
            Some(p) => p,
            None => return Ok(None),
        };
        let enum_idx = match &*entry {
            SchemaEntry::Property(PropertyKind::Byte { enum_obj, .. }) => *enum_obj,
            _ => return Ok(None),
        };
        if enum_idx == 0 {
            return Ok(None);
        }
        let pkg = self.open_package(&class_ref.stem_lc)?;
        let enum_ref = match self.resolve_index(&pkg, enum_idx)? {
            Some(r) => r,
            None => return Ok(None),
        };
        let enum_entry = self.entry(&enum_ref)?;
        if let SchemaEntry::Enum { names, .. } = &*enum_entry {
            let pkg2 = self.open_package(&enum_ref.stem_lc)?;
            return Ok(Some(
                names.iter().map(|f| pkg2.pak.fname_to_string(f)).collect(),
            ));
        }
        Ok(None)
    }

    pub fn lookup_struct_by_name(
        &self,
        starting: &str,
        name: &str,
    ) -> Result<Option<(ResolvedRef, Rc<SchemaEntry>)>> {
        for pkg_stem in [starting, "engine", "core"] {
            let pkg = match self.open_package(pkg_stem) {
                Ok(p) => p,
                Err(_) => continue,
            };
            for (idx, exp) in pkg.pak.export_table.iter().enumerate() {
                let e1 = (idx + 1) as i32;
                if pkg.pak.fname_to_string(&exp.object_name) != name {
                    continue;
                }
                let cn = pkg.export_class_name(e1);
                if cn != "ScriptStruct" && cn != "Struct" {
                    continue;
                }
                let r = ResolvedRef {
                    stem_lc: pkg.stem_lc.clone(),
                    export_idx: e1,
                };
                if let Ok(e) = self.entry(&r) {
                    return Ok(Some((r, e)));
                }
            }
        }
        Ok(None)
    }

    pub fn known_package_count(&self) -> usize {
        self.stem_index.len()
    }

    pub fn loaded_package_count(&self) -> usize {
        self.packages.borrow().len()
    }
}

trait SchemaEntryNext {
    fn common_next(&self) -> i32;
}

impl SchemaEntryNext for SchemaEntry {
    fn common_next(&self) -> i32 {
        match self {
            SchemaEntry::Property(p) => p.common().next,
            SchemaEntry::Enum { next, .. } => *next,
            SchemaEntry::Class { header, .. }
            | SchemaEntry::State { header, .. }
            | SchemaEntry::ScriptStruct { header, .. }
            | SchemaEntry::Struct { header }
            | SchemaEntry::Function { header, .. } => header.next,
        }
    }
}

fn walk_index(
    root: &Path,
    stems: &mut HashMap<String, PathBuf>,
    tfcs: &mut HashMap<String, PathBuf>,
) -> Result<()> {
    let mut q: VecDeque<PathBuf> = VecDeque::new();
    q.push_back(root.to_path_buf());
    while let Some(dir) = q.pop_front() {
        let entries = match std::fs::read_dir(&dir) {
            Ok(e) => e,
            Err(e) => {
                eprintln!("walk_index: skip {}: {}", dir.display(), e);
                continue;
            }
        };
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                q.push_back(p);
                continue;
            }
            let ext = p
                .extension()
                .and_then(|s| s.to_str())
                .map(|s| s.to_ascii_lowercase())
                .unwrap_or_default();
            let stem = p
                .file_stem()
                .and_then(|s| s.to_str())
                .map(|s| s.to_ascii_lowercase());
            match (ext.as_str(), stem) {
                ("upk" | "u" | "umap", Some(s)) => {
                    stems.entry(s).or_insert(p);
                }
                ("tfc", Some(s)) => {
                    tfcs.entry(s).or_insert(p);
                }
                _ => {}
            }
        }
    }
    Ok(())
}

pub fn open_package_at(path: &Path, stem_lc: &str) -> Result<LazyPackage> {
    let file = File::open(path)?;
    let mut reader = BufReader::new(file);

    let filesize = reader.seek(SeekFrom::End(0))?;
    reader.seek(SeekFrom::Start(0))?;

    let raw_header = UpkHeader::read(&mut reader)?;

    let (bytes, header_kept) = if raw_header.compression_method == CompressionMethod::None
        || raw_header.compressed_chunks_count == 0
    {
        reader.seek(SeekFrom::Start(0))?;
        let mut buf = Vec::with_capacity(filesize as usize);
        reader.read_to_end(&mut buf)?;
        (buf, raw_header)
    } else {
        let mut cloned = raw_header.clone();
        cloned.compression_method = CompressionMethod::None;
        cloned.compressed_chunks_count = 0;
        cloned.compressed_chunks.clear();
        cloned.pak_flags = raw_header.pak_flags & !PackageFlags::StoreCompressed.bits();

        let mut chunks = raw_header.compressed_chunks.clone();
        chunks.sort_by_key(|c| c.decompressed_offset);

        let dec_data = upk_decompress(&mut reader, raw_header.compression_method, &chunks)
            .map_err(|e| Error::new(ErrorKind::InvalidData, format!("decompress: {e}")))?;

        let mut buf: Vec<u8> = Vec::with_capacity(filesize as usize);
        {
            let mut w = Cursor::new(&mut buf);
            cloned.write(&mut w)?;
        }
        for (i, dec) in dec_data.iter().enumerate() {
            let target = chunks[i].decompressed_offset as usize;
            if buf.len() < target {
                buf.resize(target, 0);
            }
            buf.extend_from_slice(dec);
        }
        (buf, cloned)
    };

    let mut cur = Cursor::new(&bytes);
    let pak = UPKPak::parse_upk(&mut cur, &header_kept)?;

    Ok(LazyPackage {
        stem_lc: stem_lc.to_string(),
        path: path.to_path_buf(),
        bytes,
        header: header_kept,
        pak,
    })
}

fn top_package_name(pak: &UPKPak, idx: i32) -> Option<String> {
    let mut cur = idx;
    let mut guard = 0;
    while cur < 0 {
        guard += 1;
        if guard > 256 {
            return None;
        }
        let i = (-cur - 1) as usize;
        let imp = pak.import_table.get(i)?;
        if imp.outer_index == 0 {
            return Some(pak.fname_to_string(&imp.object_name));
        }
        cur = imp.outer_index;
    }
    None
}

fn collect_import_chain(pak: &UPKPak, idx: i32) -> Vec<String> {
    let mut chain = Vec::new();
    let mut cur = idx;
    let mut guard = 0;
    while cur < 0 {
        guard += 1;
        if guard > 256 {
            break;
        }
        let i = (-cur - 1) as usize;
        let imp = match pak.import_table.get(i) {
            Some(v) => v,
            None => break,
        };
        chain.push(pak.fname_to_string(&imp.object_name));
        if imp.outer_index == 0 {
            break;
        }
        cur = imp.outer_index;
    }

    chain.pop();
    chain
}

fn export_outer_chain(pkg: &LazyPackage, eidx: i32) -> Vec<String> {
    let mut chain = Vec::new();
    let mut cur = eidx;
    let mut guard = 0;
    while cur > 0 {
        guard += 1;
        if guard > 256 {
            break;
        }
        let i = (cur - 1) as usize;
        let exp = match pkg.pak.export_table.get(i) {
            Some(v) => v,
            None => break,
        };
        chain.push(pkg.pak.fname_to_string(&exp.object_name));
        if exp.outer_index == 0 {
            break;
        }
        cur = exp.outer_index;
    }
    chain
}

fn find_export_matching(
    other: &LazyPackage,
    want_name: &str,
    want_class: &str,
    want_class_pkg: &str,
    import_chain: &[String],
) -> Option<i32> {
    for (idx, exp) in other.pak.export_table.iter().enumerate() {
        let e1 = (idx + 1) as i32;
        if other.pak.fname_to_string(&exp.object_name) != want_name {
            continue;
        }
        let cname = other.export_class_name(e1);
        if !want_class.is_empty() && cname != want_class {
            continue;
        }
        if !want_class_pkg.is_empty() {
            let cpkg = export_class_package(other, e1);
            if cpkg != want_class_pkg && cpkg != "Core" {
                continue;
            }
        }
        let echain = export_outer_chain(other, e1);
        if echain != import_chain {
            continue;
        }
        return Some(e1);
    }
    None
}

fn find_export_by_name_and_outer(
    pkg: &LazyPackage,
    want_name: &str,
    want_class: &str,
    outer_export: i32,
) -> Option<i32> {
    for (idx, exp) in pkg.pak.export_table.iter().enumerate() {
        let e1 = (idx + 1) as i32;
        if pkg.pak.fname_to_string(&exp.object_name) != want_name {
            continue;
        }
        if !want_class.is_empty() && pkg.export_class_name(e1) != want_class {
            continue;
        }
        if exp.outer_index != outer_export {
            continue;
        }
        return Some(e1);
    }
    None
}

fn export_class_package(pkg: &LazyPackage, eidx: i32) -> String {
    let idx = (eidx - 1) as usize;
    let exp = match pkg.pak.export_table.get(idx) {
        Some(v) => v,
        None => return String::new(),
    };
    if exp.class_index < 0 {
        let ci = (-exp.class_index - 1) as usize;
        if let Some(class_imp) = pkg.pak.import_table.get(ci) {
            if class_imp.outer_index < 0 {
                if let Some(outer_imp) = pkg
                    .pak
                    .import_table
                    .get((-class_imp.outer_index - 1) as usize)
                {
                    return pkg.pak.fname_to_string(&outer_imp.object_name);
                }
            } else if class_imp.outer_index > 0 {
                if let Some(outer_exp) = pkg
                    .pak
                    .export_table
                    .get((class_imp.outer_index - 1) as usize)
                {
                    return pkg.pak.fname_to_string(&outer_exp.object_name);
                }
            }
            return "Core".into();
        }
    } else if exp.class_index > 0 {
        return pkg
            .path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_string();
    }
    "Core".into()
}

fn split_full_name(s: &str) -> (&str, &str) {
    match s.find(' ') {
        Some(i) => (&s[..i], &s[i + 1..]),
        None => ("", s),
    }
}

fn read_redirector_destination(pkg: &LazyPackage, eidx: i32) -> Result<Option<i32>> {
    let blob = pkg.export_blob(eidx)?.to_vec();
    let mut c = Cursor::new(&blob);
    let _net = c.read_i32::<LittleEndian>()?;

    loop {
        let before = c.position();
        let name_idx = match c.read_i32::<LittleEndian>() {
            Ok(v) => v,
            Err(_) => return Ok(None),
        };
        let _inst = c.read_i32::<LittleEndian>().unwrap_or(0);
        let nm = pkg
            .pak
            .name_table
            .get(name_idx as usize)
            .cloned()
            .unwrap_or_default();
        if nm == "None" {
            break;
        }
        c.set_position(before);
        let _ = read_fname(&mut c)?;
        let typ = read_fname(&mut c)?;
        let size = c.read_i32::<LittleEndian>()?;
        let _ai = c.read_i32::<LittleEndian>()?;
        let typ_name = pkg
            .pak
            .name_table
            .get(typ.name_index as usize)
            .cloned()
            .unwrap_or_default();
        match typ_name.as_str() {
            "StructProperty" => {
                let _ = read_fname(&mut c)?;
            }
            "BoolProperty" => {
                let _ = c.read_u8()?;
            }
            "ByteProperty" if pkg.header.p_ver >= VER_BYTEPROP_SERIALIZE_ENUM => {
                let _ = read_fname(&mut c)?;
            }
            _ => {}
        }
        c.seek(SeekFrom::Current(size as i64))?;
    }

    let dest = c.read_i32::<LittleEndian>()?;
    if dest <= 0 {
        return Ok(None);
    }
    Ok(Some(dest))
}

fn read_fname(c: &mut Cursor<&Vec<u8>>) -> Result<FName> {
    Ok(FName {
        name_index: c.read_i32::<LittleEndian>()?,
        name_instance: c.read_i32::<LittleEndian>()?,
    })
}
