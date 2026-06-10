use crate::schemadb::SchemaDb;
use crate::upkreader::{self, PackageFlags, UPKPak, UpkHeader};
use crate::utils::decompress::{CompressionMethod, upk_decompress};
use eframe::egui::{
    self, Align, Color32, FontFamily, FontId, Layout, RichText, ScrollArea, Stroke, TextStyle, Ui,
    UiKind,
};
use egui_extras::{Column, TableBuilder};
use std::collections::{BTreeMap, HashSet};
use std::fs::File;
use std::io::{BufReader, Cursor, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::rc::Rc;

pub fn run(game_root: Option<PathBuf>, verbose: bool) -> Result<(), Box<dyn std::error::Error>> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1400.0, 900.0])
            .with_min_inner_size([900.0, 540.0])
            .with_title("ue3-tools"),
        ..Default::default()
    };
    eframe::run_native(
        "ue3-tools",
        options,
        Box::new(move |cc| {
            setup_style(&cc.egui_ctx);
            let mut app = App::default();
            app.verbose = verbose;
            if let Some(gr) = game_root {
                app.preload_game_root(gr);
            }
            Ok(Box::new(app) as Box<dyn eframe::App>)
        }),
    )?;
    Ok(())
}

fn setup_style(ctx: &egui::Context) {
    ctx.set_visuals(dnspy_dark_visuals());
    let mut s = (*ctx.global_style()).clone();
    s.text_styles = [
        (
            TextStyle::Heading,
            FontId::new(15.0, FontFamily::Proportional),
        ),
        (TextStyle::Body, FontId::new(13.0, FontFamily::Proportional)),
        (
            TextStyle::Monospace,
            FontId::new(12.5, FontFamily::Monospace),
        ),
        (
            TextStyle::Button,
            FontId::new(13.0, FontFamily::Proportional),
        ),
        (
            TextStyle::Small,
            FontId::new(11.0, FontFamily::Proportional),
        ),
    ]
    .into();
    s.spacing.item_spacing = egui::vec2(6.0, 4.0);
    s.spacing.button_padding = egui::vec2(8.0, 3.0);
    s.spacing.window_margin = egui::Margin::same(6);
    ctx.set_global_style(s);
}

fn dnspy_dark_visuals() -> egui::Visuals {
    let mut v = egui::Visuals::dark();
    v.panel_fill = Color32::from_rgb(0x1e, 0x1e, 0x1e);
    v.window_fill = Color32::from_rgb(0x25, 0x25, 0x26);
    v.faint_bg_color = Color32::from_rgb(0x2d, 0x2d, 0x30);
    v.extreme_bg_color = Color32::from_rgb(0x14, 0x14, 0x14);
    v.code_bg_color = Color32::from_rgb(0x1e, 0x1e, 0x1e);
    v.widgets.noninteractive.bg_fill = Color32::from_rgb(0x25, 0x25, 0x26);
    v.widgets.inactive.bg_fill = Color32::from_rgb(0x33, 0x33, 0x37);
    v.widgets.inactive.fg_stroke = Stroke::new(1.0, Color32::from_rgb(0xdc, 0xdc, 0xdc));
    v.widgets.hovered.bg_fill = Color32::from_rgb(0x3e, 0x3e, 0x42);
    v.widgets.hovered.fg_stroke = Stroke::new(1.0, Color32::WHITE);
    v.widgets.active.bg_fill = Color32::from_rgb(0x00, 0x7a, 0xcc);
    v.selection.bg_fill = Color32::from_rgb(0x09, 0x4c, 0x8b);
    v.selection.stroke = Stroke::new(1.0, Color32::from_rgb(0x4e, 0xc9, 0xff));
    v.hyperlink_color = Color32::from_rgb(0x4e, 0xc9, 0xff);
    v
}

fn class_color(name: &str) -> Color32 {
    match name {
        "Class" => Color32::from_rgb(0xb8, 0xd7, 0xa3),
        "Function" | "ScriptFunction" => Color32::from_rgb(0xdc, 0xdc, 0xaa),
        "State" => Color32::from_rgb(0xc5, 0x86, 0xc0),
        "Enum" | "Const" => Color32::from_rgb(0xb5, 0xce, 0xa8),
        "ScriptStruct" | "Struct" => Color32::from_rgb(0x4e, 0xc9, 0xb0),
        "Texture2D" | "Texture" | "TextureCube" => Color32::from_rgb(0xff, 0xd7, 0x00),
        "Font" => Color32::from_rgb(0xff, 0xa5, 0x00),
        "Material" | "MaterialInstance" | "MaterialInstanceConstant" => {
            Color32::from_rgb(0xff, 0x6e, 0xc7)
        }
        "Package" => Color32::from_rgb(0x9c, 0xdc, 0xfe),
        s if s.ends_with("Property") => Color32::from_rgb(0xce, 0x91, 0x78),
        _ => Color32::from_rgb(0xdc, 0xdc, 0xdc),
    }
}

fn class_glyph(name: &str) -> &'static str {
    match name {
        "Class" => "C",
        "Function" | "ScriptFunction" => "ƒ",
        "State" => "§",
        "Enum" => "E",
        "Const" => "κ",
        "ScriptStruct" | "Struct" => "S",
        "Texture2D" | "Texture" | "TextureCube" => "T",
        "Font" => "A",
        "Material" => "M",
        "Package" => "P",
        s if s.ends_with("Property") => "•",
        _ => "◆",
    }
}

struct LoadedUpk {
    path: PathBuf,
    name: String,
    bytes: Vec<u8>,
    header: UpkHeader,
    pak: UPKPak,

    classes: BTreeMap<String, Vec<i32>>,
    expanded_classes: HashSet<String>,
    expanded_exports: bool,
    expanded_imports: bool,
    expanded_names: bool,
}

impl LoadedUpk {
    fn load(path: &Path) -> Result<Self, String> {
        let mut f = BufReader::new(File::open(path).map_err(|e| e.to_string())?);
        let filesize = f.seek(SeekFrom::End(0)).map_err(|e| e.to_string())?;
        f.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
        let header = UpkHeader::read(&mut f).map_err(|e| e.to_string())?;

        let bytes = if header.compression_method == CompressionMethod::None
            || header.compressed_chunks_count == 0
        {
            f.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
            let mut buf = Vec::with_capacity(filesize as usize);
            f.read_to_end(&mut buf).map_err(|e| e.to_string())?;
            buf
        } else {
            decompress_to_memory(&mut f, &header, filesize)?
        };

        let final_header = {
            let mut c = Cursor::new(&bytes);
            UpkHeader::read(&mut c).map_err(|e| e.to_string())?
        };
        let mut cur = Cursor::new(&bytes);
        let pak = UPKPak::parse_upk(&mut cur, &final_header).map_err(|e| e.to_string())?;

        let mut classes: BTreeMap<String, Vec<i32>> = BTreeMap::new();
        for (i, exp) in pak.export_table.iter().enumerate() {
            let class_name = pak.get_class_name(exp.class_index);
            classes.entry(class_name).or_default().push((i + 1) as i32);
        }
        for v in classes.values_mut() {
            v.sort_by(|a, b| {
                let na = pak.fname_to_string(&pak.export_table[(*a - 1) as usize].object_name);
                let nb = pak.fname_to_string(&pak.export_table[(*b - 1) as usize].object_name);
                na.to_lowercase().cmp(&nb.to_lowercase())
            });
        }

        let name = path
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();

        Ok(Self {
            path: path.to_path_buf(),
            name,
            bytes,
            header: final_header,
            pak,
            classes,
            expanded_classes: HashSet::new(),
            expanded_exports: true,
            expanded_imports: false,
            expanded_names: false,
        })
    }
}

fn decompress_to_memory(
    f: &mut BufReader<File>,
    header: &UpkHeader,
    filesize: u64,
) -> Result<Vec<u8>, String> {
    let mut cloned = header.clone();
    cloned.compression_method = CompressionMethod::None;
    cloned.compressed_chunks_count = 0;
    cloned.compressed_chunks.clear();
    cloned.pak_flags = header.pak_flags & !PackageFlags::StoreCompressed.bits();

    let mut chunks = header.compressed_chunks.clone();
    chunks.sort_by_key(|c| c.decompressed_offset);
    let dec = upk_decompress(&mut *f, header.compression_method, &chunks)
        .map_err(|e| format!("decompression error: {e:?}"))?;

    let dec_total = chunks
        .iter()
        .zip(dec.iter())
        .map(|(c, d)| c.decompressed_offset as usize + d.len())
        .max()
        .unwrap_or(0);

    let mut buf: Vec<u8> = Vec::with_capacity(dec_total.max(filesize as usize));
    {
        let mut w = std::io::Cursor::new(&mut buf);
        cloned.write(&mut w).map_err(|e| e.to_string())?;
    }
    for (i, d) in dec.iter().enumerate() {
        if i != 0 {
            let prev = chunks[i - 1].compressed_offset + chunks[i - 1].compressed_size;
            let gap = chunks[i].compressed_offset.saturating_sub(prev);
            if gap > 0 {
                f.seek(SeekFrom::Start(prev as u64))
                    .map_err(|e| e.to_string())?;
                let mut gb = vec![0u8; gap as usize];
                f.read_exact(&mut gb).map_err(|e| e.to_string())?;
                buf.extend_from_slice(&gb);
            }
        }
        let target = chunks[i].decompressed_offset as usize;
        if buf.len() < target {
            buf.resize(target, 0);
        } else if buf.len() > target {
            buf[target..target + d.len()].copy_from_slice(d);
            continue;
        }
        buf.extend_from_slice(d);
    }
    let last_end = chunks
        .last()
        .map(|c| (c.compressed_offset + c.compressed_size) as u64)
        .unwrap_or(0);
    if filesize > last_end {
        f.seek(SeekFrom::Start(last_end))
            .map_err(|e| e.to_string())?;
        let mut tail = Vec::with_capacity((filesize - last_end) as usize);
        f.read_to_end(&mut tail).map_err(|e| e.to_string())?;
        buf.extend_from_slice(&tail);
    }
    Ok(buf)
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum TabKind {
    Header,
    Names,
    Imports,
    Exports,
    Export(i32),
    Import(i32),
}

#[derive(Clone)]
struct Tab {
    pkg: usize,
    kind: TabKind,
    title: String,
}

struct App {
    workspace: Vec<LoadedUpk>,
    tabs: Vec<Tab>,
    active_tab: Option<usize>,
    log: Vec<LogLine>,
    show_log: bool,
    game_root: Option<PathBuf>,
    schema_db: Option<Rc<SchemaDb>>,
    filter: String,
    verbose: bool,
}

impl Default for App {
    fn default() -> Self {
        let mut s = Self {
            workspace: Vec::new(),
            tabs: Vec::new(),
            active_tab: None,
            log: Vec::new(),
            show_log: true,
            game_root: None,
            schema_db: None,
            filter: String::new(),
            verbose: false,
        };
        s.log_info("ue3-tools UI ready. File → Open… to load a .upk.");
        s
    }
}

#[derive(Clone)]
struct LogLine {
    level: LogLevel,
    text: String,
}

#[derive(Clone, Copy)]
enum LogLevel {
    Info,
    Warn,
    Error,
}

impl LogLevel {
    fn color(self) -> Color32 {
        match self {
            LogLevel::Info => Color32::from_rgb(0xcc, 0xcc, 0xcc),
            LogLevel::Warn => Color32::from_rgb(0xff, 0xc4, 0x66),
            LogLevel::Error => Color32::from_rgb(0xf4, 0x47, 0x47),
        }
    }
    fn tag(self) -> &'static str {
        match self {
            LogLevel::Info => "INFO",
            LogLevel::Warn => "WARN",
            LogLevel::Error => "ERR ",
        }
    }
}

enum Action {
    OpenFile(PathBuf),
    OpenGameRoot(PathBuf),
    ClosePackage(usize),
    CloseAll,
    OpenTab(Tab),
    CloseTab(usize),
    ActivateTab(usize),
    ToggleExpandClass(usize, String),
    ToggleExpand(usize, &'static str),
    Log(LogLevel, String),
    Quit,
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let mut actions: Vec<Action> = Vec::new();

        self.menu_bar(ui, &mut actions);
        self.toolbar(ui, &mut actions);
        self.status_bar(ui);
        if self.show_log {
            self.log_panel(ui);
        }
        self.tree_panel(ui, &mut actions);
        self.center(ui, &mut actions);

        for a in actions {
            self.apply(a, ui);
        }
    }
}

impl App {
    fn preload_game_root(&mut self, p: PathBuf) {
        match SchemaDb::new(&p) {
            Ok(db) => {
                let db = db.with_verbose(self.verbose);
                let n = db.known_package_count();
                self.log_info(format!(
                    "indexed {n} package(s) in game root  {}",
                    p.display()
                ));
                self.game_root = Some(p);
                self.schema_db = Some(Rc::new(db));
            }
            Err(e) => self.log_err(format!("game root index failed: {e}")),
        }
    }
    fn log_info(&mut self, s: impl Into<String>) {
        self.log.push(LogLine {
            level: LogLevel::Info,
            text: s.into(),
        });
    }
    fn log_warn(&mut self, s: impl Into<String>) {
        self.log.push(LogLine {
            level: LogLevel::Warn,
            text: s.into(),
        });
    }
    fn log_err(&mut self, s: impl Into<String>) {
        self.log.push(LogLine {
            level: LogLevel::Error,
            text: s.into(),
        });
    }

    fn apply(&mut self, a: Action, ui: &mut Ui) {
        match a {
            Action::OpenFile(p) => match LoadedUpk::load(&p) {
                Ok(pkg) => {
                    self.log_info(format!(
                        "loaded {}  (p_ver={}, names={}, exports={}, imports={})",
                        pkg.name,
                        pkg.header.p_ver,
                        pkg.header.name_count,
                        pkg.header.export_count,
                        pkg.header.import_count
                    ));
                    let idx = self.workspace.len();
                    self.workspace.push(pkg);
                    self.open_tab(Tab {
                        pkg: idx,
                        kind: TabKind::Header,
                        title: format!("{} · header", self.workspace[idx].name),
                    });
                }
                Err(e) => self.log_err(format!("load failed: {e}")),
            },
            Action::OpenGameRoot(p) => match SchemaDb::new(&p) {
                Ok(db) => {
                    let db = db.with_verbose(self.verbose);
                    let n = db.known_package_count();
                    self.game_root = Some(p);
                    self.schema_db = Some(Rc::new(db));
                    self.log_info(format!("indexed {n} package(s) in game root"));
                }
                Err(e) => self.log_err(format!("game root index failed: {e}")),
            },
            Action::ClosePackage(i) => {
                if i < self.workspace.len() {
                    let name = self.workspace[i].name.clone();
                    self.workspace.remove(i);
                    self.tabs.retain(|t| t.pkg != i);
                    for t in &mut self.tabs {
                        if t.pkg > i {
                            t.pkg -= 1;
                        }
                    }
                    if let Some(a) = self.active_tab {
                        if a >= self.tabs.len() {
                            self.active_tab = self.tabs.len().checked_sub(1);
                        }
                    }
                    self.log_info(format!("closed {name}"));
                }
            }
            Action::CloseAll => {
                self.workspace.clear();
                self.tabs.clear();
                self.active_tab = None;
                self.log_info("closed all packages");
            }
            Action::OpenTab(t) => self.open_tab(t),
            Action::CloseTab(i) => {
                if i < self.tabs.len() {
                    self.tabs.remove(i);
                    if let Some(a) = self.active_tab {
                        if a == i {
                            self.active_tab = if self.tabs.is_empty() {
                                None
                            } else {
                                Some(i.saturating_sub(1).min(self.tabs.len() - 1))
                            };
                        } else if a > i {
                            self.active_tab = Some(a - 1);
                        }
                    }
                }
            }
            Action::ActivateTab(i) => {
                if i < self.tabs.len() {
                    self.active_tab = Some(i);
                }
            }
            Action::ToggleExpandClass(pkg, c) => {
                if let Some(p) = self.workspace.get_mut(pkg) {
                    if !p.expanded_classes.remove(&c) {
                        p.expanded_classes.insert(c);
                    }
                }
            }
            Action::ToggleExpand(pkg, which) => {
                if let Some(p) = self.workspace.get_mut(pkg) {
                    match which {
                        "exports" => p.expanded_exports = !p.expanded_exports,
                        "imports" => p.expanded_imports = !p.expanded_imports,
                        "names" => p.expanded_names = !p.expanded_names,
                        _ => {}
                    }
                }
            }
            Action::Log(lvl, s) => self.log.push(LogLine {
                level: lvl,
                text: s,
            }),
            Action::Quit => ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close),
        }
    }

    fn open_tab(&mut self, t: Tab) {
        if let Some(i) = self
            .tabs
            .iter()
            .position(|x| x.pkg == t.pkg && x.kind == t.kind)
        {
            self.active_tab = Some(i);
        } else {
            self.tabs.push(t);
            self.active_tab = Some(self.tabs.len() - 1);
        }
    }
}

impl App {
    fn menu_bar(&mut self, ui: &mut Ui, actions: &mut Vec<Action>) {
        egui::Panel::top("menubar").show_inside(ui, |ui| {
            egui::MenuBar::new().ui(ui, |ui| {
                ui.menu_button("File", |ui| {
                    if ui.button("Open .upk…").clicked() {
                        if let Some(p) = rfd::FileDialog::new()
                            .add_filter("UE3 package", &["upk", "Package", "u"])
                            .add_filter("All files", &["*"])
                            .pick_file()
                        {
                            actions.push(Action::OpenFile(p));
                        }
                        ui.close_kind(UiKind::Menu);
                    }
                    if ui.button("Set game root…").clicked() {
                        if let Some(p) = rfd::FileDialog::new().pick_folder() {
                            actions.push(Action::OpenGameRoot(p));
                        }
                        ui.close_kind(UiKind::Menu);
                    }
                    ui.separator();
                    if ui.button("Close all").clicked() {
                        actions.push(Action::CloseAll);
                        ui.close_kind(UiKind::Menu);
                    }
                    ui.separator();
                    if ui.button("Quit").clicked() {
                        actions.push(Action::Quit);
                        ui.close_kind(UiKind::Menu);
                    }
                });
                ui.menu_button("View", |ui| {
                    ui.checkbox(&mut self.show_log, "Show output log");
                    ui.checkbox(&mut self.verbose, "Verbose schema");
                });
                ui.menu_button("Help", |ui| {
                    ui.label(RichText::new("ue3-tools").strong());
                    ui.label("UE3 UPK toolkit · egui front-end");
                });
            });
        });
    }

    fn toolbar(&mut self, ui: &mut Ui, actions: &mut Vec<Action>) {
        egui::Panel::top("toolbar").show_inside(ui, |ui| {
            ui.horizontal(|ui| {
                if ui.button("📂 Open").clicked() {
                    if let Some(p) = rfd::FileDialog::new()
                        .add_filter("UE3 package", &["upk", "Package", "u"])
                        .pick_file()
                    {
                        actions.push(Action::OpenFile(p));
                    }
                }
                if ui.button("🗁 Game Root").clicked() {
                    if let Some(p) = rfd::FileDialog::new().pick_folder() {
                        actions.push(Action::OpenGameRoot(p));
                    }
                }
                ui.separator();
                let gr_text = match &self.game_root {
                    Some(p) => format!("root: {}", p.display()),
                    None => "root: (not set)".to_string(),
                };
                ui.label(RichText::new(gr_text).weak().small());

                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.add_sized(
                        [220.0, 22.0],
                        egui::TextEdit::singleline(&mut self.filter).hint_text("filter…"),
                    );
                    ui.label("🔎");
                });
            });
        });
    }

    fn status_bar(&self, ui: &mut Ui) {
        egui::Panel::bottom("status").show_inside(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new(format!("{} package(s)", self.workspace.len())).small());
                ui.separator();
                if let Some(i) = self.active_tab {
                    if let Some(t) = self.tabs.get(i) {
                        if let Some(p) = self.workspace.get(t.pkg) {
                            ui.label(
                                RichText::new(format!(
                                    "p_ver={} · names={} · exports={} · imports={}",
                                    p.header.p_ver,
                                    p.header.name_count,
                                    p.header.export_count,
                                    p.header.import_count
                                ))
                                .small(),
                            );
                        }
                    }
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if let Some(g) = &self.game_root {
                        ui.label(
                            RichText::new(format!("schema-db ✓ {}", g.display()))
                                .small()
                                .color(Color32::from_rgb(0x8b, 0xc3, 0x4a)),
                        );
                    } else {
                        ui.label(RichText::new("schema-db: off").small().weak());
                    }
                });
            });
        });
    }

    fn log_panel(&self, ui: &mut Ui) {
        egui::Panel::bottom("log")
            .resizable(true)
            .default_size(140.0)
            .show_inside(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Output").strong());
                });
                ui.separator();
                ScrollArea::vertical().stick_to_bottom(true).show(ui, |ui| {
                    for line in &self.log {
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new(format!("[{}]", line.level.tag()))
                                    .monospace()
                                    .color(line.level.color())
                                    .small(),
                            );
                            ui.label(RichText::new(&line.text).monospace().small());
                        });
                    }
                });
            });
    }
}

impl App {
    fn tree_panel(&mut self, ui: &mut Ui, actions: &mut Vec<Action>) {
        egui::Panel::left("tree")
            .resizable(true)
            .default_size(320.0)
            .size_range(220.0..=600.0)
            .show_inside(ui, |ui| {
                ui.add_space(2.0);
                ui.label(RichText::new("WORKSPACE").small().weak());
                ui.separator();

                ScrollArea::vertical()
                    .auto_shrink([false; 2])
                    .show(ui, |ui| {
                        let filter_lc = self.filter.to_lowercase();
                        for pi in 0..self.workspace.len() {
                            self.render_package_node(ui, pi, &filter_lc, actions);
                        }
                        if self.workspace.is_empty() {
                            ui.add_space(8.0);
                            ui.label(
                                RichText::new("No packages loaded.\nFile → Open .upk…").weak(),
                            );
                        }
                    });
            });
    }

    fn render_package_node(
        &self,
        ui: &mut Ui,
        pi: usize,
        filter_lc: &str,
        actions: &mut Vec<Action>,
    ) {
        let pkg = &self.workspace[pi];

        let id = ui.make_persistent_id(("pkg", pi));
        egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, true)
            .show_header(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("📦").color(class_color("Package")));
                    let r = ui.selectable_label(false, RichText::new(&pkg.name).strong());
                    if r.clicked() {
                        actions.push(Action::OpenTab(Tab {
                            pkg: pi,
                            kind: TabKind::Header,
                            title: format!("{} · header", pkg.name),
                        }));
                    }
                    r.context_menu(|ui| {
                        if ui.button("Close package").clicked() {
                            actions.push(Action::ClosePackage(pi));
                            ui.close_kind(UiKind::Menu);
                        }
                    });
                });
            })
            .body(|ui| {
                if ui.selectable_label(false, "ⓘ  Header").clicked() {
                    actions.push(Action::OpenTab(Tab {
                        pkg: pi,
                        kind: TabKind::Header,
                        title: format!("{} · header", pkg.name),
                    }));
                }
                if ui
                    .selectable_label(false, format!("🔤  Names  ({})", pkg.header.name_count))
                    .clicked()
                {
                    actions.push(Action::OpenTab(Tab {
                        pkg: pi,
                        kind: TabKind::Names,
                        title: format!("{} · names", pkg.name),
                    }));
                }
                if ui
                    .selectable_label(false, format!("↓  Imports  ({})", pkg.header.import_count))
                    .clicked()
                {
                    actions.push(Action::OpenTab(Tab {
                        pkg: pi,
                        kind: TabKind::Imports,
                        title: format!("{} · imports", pkg.name),
                    }));
                }
                if ui
                    .selectable_label(false, format!("↑  Exports  ({})", pkg.header.export_count))
                    .clicked()
                {
                    actions.push(Action::OpenTab(Tab {
                        pkg: pi,
                        kind: TabKind::Exports,
                        title: format!("{} · exports", pkg.name),
                    }));
                }

                ui.add_space(4.0);
                ui.label(RichText::new("BY CLASS").small().weak());
                ui.separator();

                for (class_name, idxs) in &pkg.classes {
                    if !filter_lc.is_empty()
                        && !class_name.to_lowercase().contains(filter_lc)
                        && !idxs.iter().any(|i| {
                            let n = pkg.fname_to_string_safe(*i);
                            n.to_lowercase().contains(filter_lc)
                        })
                    {
                        continue;
                    }

                    let col = class_color(class_name);
                    let header_id = ui.make_persistent_id(("cls", pi, class_name));
                    egui::collapsing_header::CollapsingState::load_with_default_open(
                        ui.ctx(),
                        header_id,
                        false,
                    )
                    .show_header(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new(class_glyph(class_name))
                                    .color(col)
                                    .monospace(),
                            );
                            ui.label(
                                RichText::new(format!("{}  [{}]", class_name, idxs.len()))
                                    .color(col),
                            );
                        });
                    })
                    .body(|ui| {
                        for &idx in idxs {
                            let name = pkg.fname_to_string_safe(idx);
                            if !filter_lc.is_empty()
                                && !name.to_lowercase().contains(filter_lc)
                                && !class_name.to_lowercase().contains(filter_lc)
                            {
                                continue;
                            }
                            let label = RichText::new(format!("  {}", name)).color(col);
                            if ui.selectable_label(false, label).clicked() {
                                actions.push(Action::OpenTab(Tab {
                                    pkg: pi,
                                    kind: TabKind::Export(idx),
                                    title: format!("{}", name),
                                }));
                            }
                        }
                    });
                }
            });
    }
}

impl LoadedUpk {
    fn fname_to_string_safe(&self, export_1based: i32) -> String {
        let i = (export_1based - 1) as usize;
        self.pak
            .export_table
            .get(i)
            .map(|e| self.pak.fname_to_string(&e.object_name))
            .unwrap_or_else(|| format!("?#{export_1based}"))
    }
}

impl App {
    fn center(&mut self, ui: &mut Ui, actions: &mut Vec<Action>) {
        egui::CentralPanel::default().show_inside(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                let active = self.active_tab;
                for (i, t) in self.tabs.iter().enumerate() {
                    let is_active = Some(i) == active;
                    let bg = if is_active {
                        Color32::from_rgb(0x1e, 0x1e, 0x1e)
                    } else {
                        Color32::from_rgb(0x2d, 0x2d, 0x30)
                    };
                    let fg = if is_active {
                        Color32::WHITE
                    } else {
                        Color32::from_rgb(0xbb, 0xbb, 0xbb)
                    };
                    egui::Frame::NONE
                        .fill(bg)
                        .stroke(Stroke::new(1.0, Color32::from_rgb(0x3f, 0x3f, 0x46)))
                        .inner_margin(egui::Margin::symmetric(8, 3))
                        .show(ui, |ui| {
                            let r = ui.add(
                                egui::Label::new(RichText::new(&t.title).color(fg))
                                    .sense(egui::Sense::click()),
                            );
                            if r.clicked() {
                                actions.push(Action::ActivateTab(i));
                            }
                            ui.add_space(4.0);
                            if ui
                                .add(
                                    egui::Label::new(
                                        RichText::new("✕")
                                            .color(Color32::from_rgb(0x88, 0x88, 0x88))
                                            .small(),
                                    )
                                    .sense(egui::Sense::click()),
                                )
                                .clicked()
                            {
                                actions.push(Action::CloseTab(i));
                            }
                        });
                }
            });
            ui.separator();

            if let Some(i) = self.active_tab {
                if let Some(t) = self.tabs.get(i).cloned() {
                    self.render_tab(ui, &t, actions);
                }
            } else {
                ui.vertical_centered(|ui| {
                    ui.add_space(40.0);
                    ui.label(RichText::new("No tab open").weak().size(16.0));
                    ui.label(
                        RichText::new("Click items in the workspace tree on the left.").weak(),
                    );
                });
            }
        });
    }
    fn render_tab(&self, ui: &mut Ui, t: &Tab, actions: &mut Vec<Action>) {
        let pkg = match self.workspace.get(t.pkg) {
            Some(p) => p,
            None => {
                ui.label(RichText::new("(package no longer loaded)").weak());
                return;
            }
        };
        match t.kind {
            TabKind::Header => self.view_header(ui, pkg),
            TabKind::Names => self.view_names(ui, pkg),
            TabKind::Imports => self.view_imports(ui, pkg, t.pkg, actions),
            TabKind::Exports => self.view_exports(ui, pkg, t.pkg, actions),
            TabKind::Export(idx) => self.view_export(ui, pkg, t.pkg, idx, actions),
            TabKind::Import(idx) => self.view_import(ui, pkg, t.pkg, idx, actions),
        }
    }

    fn view_header(&self, ui: &mut Ui, pkg: &LoadedUpk) {
        ScrollArea::vertical()
            .auto_shrink([false; 2])
            .show(ui, |ui| {
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.label(RichText::new("📦").color(class_color("Package")).size(18.0));
                    ui.heading(&pkg.name);
                });
                ui.label(
                    RichText::new(pkg.path.display().to_string())
                        .weak()
                        .small()
                        .monospace(),
                );
                ui.add_space(8.0);

                let h = &pkg.header;
                let rows: Vec<(&str, String)> = vec![
                    ("signature", format!("0x{:08x}", h.sign)),
                    ("p_ver / l_ver", format!("{} / {}", h.p_ver, h.l_ver)),
                    (
                        "header_size",
                        format!("{}  (0x{:x})", h.header_size, h.header_size),
                    ),
                    (
                        "path",
                        String::from_utf8_lossy(&h.path)
                            .trim_end_matches('\0')
                            .to_string(),
                    ),
                    ("pak_flags", format!("0x{:08x}", h.pak_flags)),
                    ("name_count", h.name_count.to_string()),
                    ("name_offset", format!("0x{:x}", h.name_offset)),
                    ("export_count", h.export_count.to_string()),
                    ("export_offset", format!("0x{:x}", h.export_offset)),
                    ("import_count", h.import_count.to_string()),
                    ("import_offset", format!("0x{:x}", h.import_offset)),
                    ("depends_offset", format!("0x{:x}", h.depends_offset)),
                    ("engine_ver", h.engine_ver.to_string()),
                    ("cooker_ver", h.cooker_ver.to_string()),
                    ("compression", format!("{:?}", h.compression_method)),
                    ("compressed_chunks", h.compressed_chunks_count.to_string()),
                    (
                        "package_source",
                        format!("0x{:08x}", h.package_source as u32),
                    ),
                    (
                        "guid",
                        format!(
                            "{:08x}-{:08x}-{:08x}-{:08x}",
                            h.guid[0] as u32, h.guid[1] as u32, h.guid[2] as u32, h.guid[3] as u32,
                        ),
                    ),
                    (
                        "file size",
                        format!(
                            "{} bytes  ({:.2} MiB)",
                            pkg.bytes.len(),
                            pkg.bytes.len() as f64 / (1024.0 * 1024.0)
                        ),
                    ),
                ];

                TableBuilder::new(ui)
                    .striped(true)
                    .resizable(true)
                    .column(Column::auto().at_least(180.0))
                    .column(Column::remainder())
                    .header(22.0, |mut h| {
                        h.col(|ui| {
                            ui.strong("Field");
                        });
                        h.col(|ui| {
                            ui.strong("Value");
                        });
                    })
                    .body(|mut body| {
                        for (k, v) in &rows {
                            body.row(20.0, |mut row| {
                                row.col(|ui| {
                                    ui.label(
                                        RichText::new(*k)
                                            .monospace()
                                            .color(Color32::from_rgb(0x9c, 0xdc, 0xfe)),
                                    );
                                });
                                row.col(|ui| {
                                    ui.label(RichText::new(v).monospace());
                                });
                            });
                        }
                    });
            });
    }

    fn view_names(&self, ui: &mut Ui, pkg: &LoadedUpk) {
        ui.horizontal(|ui| {
            ui.label(RichText::new(format!("Names — {}", pkg.header.name_count)).strong());
            ui.label(
                RichText::new(format!("· offset 0x{:x}", pkg.header.name_offset))
                    .weak()
                    .small(),
            );
        });
        ui.separator();

        let filter_lc = self.filter.to_lowercase();
        let names = &pkg.pak.name_table;
        let filtered: Vec<usize> = if filter_lc.is_empty() {
            (0..names.len()).collect()
        } else {
            (0..names.len())
                .filter(|&i| names[i].to_lowercase().contains(&filter_lc))
                .collect()
        };

        TableBuilder::new(ui)
            .striped(true)
            .resizable(true)
            .column(Column::auto().at_least(60.0))
            .column(Column::remainder())
            .header(22.0, |mut h| {
                h.col(|ui| {
                    ui.strong("#");
                });
                h.col(|ui| {
                    ui.strong("name");
                });
            })
            .body(|body| {
                body.rows(18.0, filtered.len(), |mut row| {
                    let row_i = row.index();
                    let i = filtered[row_i];
                    row.col(|ui| {
                        ui.label(RichText::new(format!("{i:>5}")).monospace().weak());
                    });
                    row.col(|ui| {
                        ui.label(RichText::new(&names[i]).monospace());
                    });
                });
            });
    }

    fn view_imports(&self, ui: &mut Ui, pkg: &LoadedUpk, pi: usize, actions: &mut Vec<Action>) {
        ui.horizontal(|ui| {
            ui.label(RichText::new(format!("Imports — {}", pkg.header.import_count)).strong());
        });
        ui.separator();

        let filter_lc = self.filter.to_lowercase();
        let total = pkg.pak.import_table.len();
        let visible: Vec<usize> = if filter_lc.is_empty() {
            (0..total).collect()
        } else {
            (0..total)
                .filter(|&i| {
                    let imp = &pkg.pak.import_table[i];
                    pkg.pak
                        .fname_to_string(&imp.object_name)
                        .to_lowercase()
                        .contains(&filter_lc)
                        || pkg
                            .pak
                            .fname_to_string(&imp.class_name)
                            .to_lowercase()
                            .contains(&filter_lc)
                        || pkg
                            .pak
                            .fname_to_string(&imp.class_package)
                            .to_lowercase()
                            .contains(&filter_lc)
                })
                .collect()
        };

        TableBuilder::new(ui)
            .striped(true)
            .resizable(true)
            .column(Column::auto().at_least(60.0))
            .column(Column::auto().at_least(120.0))
            .column(Column::auto().at_least(140.0))
            .column(Column::auto().at_least(60.0))
            .column(Column::remainder())
            .header(22.0, |mut h| {
                h.col(|ui| {
                    ui.strong("#");
                });
                h.col(|ui| {
                    ui.strong("class_pkg");
                });
                h.col(|ui| {
                    ui.strong("class");
                });
                h.col(|ui| {
                    ui.strong("outer");
                });
                h.col(|ui| {
                    ui.strong("object");
                });
            })
            .body(|body| {
                body.rows(18.0, visible.len(), |mut row| {
                    let r = row.index();
                    let i = visible[r];
                    let imp = &pkg.pak.import_table[i];
                    let one_based = -((i as i32) + 1);

                    row.col(|ui| {
                        ui.label(
                            RichText::new(format!("{:>5}", one_based))
                                .monospace()
                                .weak(),
                        );
                    });
                    row.col(|ui| {
                        ui.label(
                            RichText::new(pkg.pak.fname_to_string(&imp.class_package)).monospace(),
                        );
                    });
                    row.col(|ui| {
                        let cls = pkg.pak.fname_to_string(&imp.class_name);
                        ui.label(
                            RichText::new(cls.clone())
                                .monospace()
                                .color(class_color(&cls)),
                        );
                    });
                    row.col(|ui| {
                        ui.label(
                            RichText::new(format!("{}", imp.outer_index))
                                .monospace()
                                .weak(),
                        );
                    });
                    row.col(|ui| {
                        let n = pkg.pak.fname_to_string(&imp.object_name);
                        let r = ui.add(
                            egui::Label::new(RichText::new(&n).monospace())
                                .sense(egui::Sense::click()),
                        );
                        if r.clicked() {
                            actions.push(Action::OpenTab(Tab {
                                pkg: pi,
                                kind: TabKind::Import((i as i32) + 1),
                                title: format!("⤓ {n}"),
                            }));
                        }
                    });
                });
            });
    }

    fn view_exports(&self, ui: &mut Ui, pkg: &LoadedUpk, pi: usize, actions: &mut Vec<Action>) {
        ui.horizontal(|ui| {
            ui.label(RichText::new(format!("Exports — {}", pkg.header.export_count)).strong());
        });
        ui.separator();

        let filter_lc = self.filter.to_lowercase();
        let total = pkg.pak.export_table.len();
        let visible: Vec<usize> = if filter_lc.is_empty() {
            (0..total).collect()
        } else {
            (0..total)
                .filter(|&i| {
                    let e = &pkg.pak.export_table[i];
                    let n = pkg.pak.fname_to_string(&e.object_name);
                    let c = pkg.pak.get_class_name(e.class_index);
                    n.to_lowercase().contains(&filter_lc) || c.to_lowercase().contains(&filter_lc)
                })
                .collect()
        };

        TableBuilder::new(ui)
            .striped(true)
            .resizable(true)
            .column(Column::auto().at_least(60.0))
            .column(Column::auto().at_least(150.0))
            .column(Column::remainder())
            .column(Column::auto().at_least(90.0))
            .column(Column::auto().at_least(90.0))
            .header(22.0, |mut h| {
                h.col(|ui| {
                    ui.strong("#");
                });
                h.col(|ui| {
                    ui.strong("class");
                });
                h.col(|ui| {
                    ui.strong("name");
                });
                h.col(|ui| {
                    ui.strong("offset");
                });
                h.col(|ui| {
                    ui.strong("size");
                });
            })
            .body(|body| {
                body.rows(18.0, visible.len(), |mut row| {
                    let r = row.index();
                    let i = visible[r];
                    let e = &pkg.pak.export_table[i];
                    let one_based = (i as i32) + 1;
                    let class = pkg.pak.get_class_name(e.class_index);
                    let name = pkg.pak.fname_to_string(&e.object_name);

                    row.col(|ui| {
                        ui.label(RichText::new(format!("{one_based:>5}")).monospace().weak());
                    });
                    row.col(|ui| {
                        ui.label(
                            RichText::new(class.clone())
                                .monospace()
                                .color(class_color(&class)),
                        );
                    });
                    row.col(|ui| {
                        let r = ui.add(
                            egui::Label::new(
                                RichText::new(&name).monospace().color(class_color(&class)),
                            )
                            .sense(egui::Sense::click()),
                        );
                        if r.clicked() {
                            actions.push(Action::OpenTab(Tab {
                                pkg: pi,
                                kind: TabKind::Export(one_based),
                                title: name.clone(),
                            }));
                        }
                    });
                    row.col(|ui| {
                        ui.label(
                            RichText::new(format!("0x{:x}", e.serial_offset))
                                .monospace()
                                .weak(),
                        );
                    });
                    row.col(|ui| {
                        ui.label(
                            RichText::new(format!("{}", e.serial_size))
                                .monospace()
                                .weak(),
                        );
                    });
                });
            });
    }

    fn view_export(
        &self,
        ui: &mut Ui,
        pkg: &LoadedUpk,
        pi: usize,
        one_based: i32,
        actions: &mut Vec<Action>,
    ) {
        let idx = (one_based - 1) as usize;
        let e = match pkg.pak.export_table.get(idx) {
            Some(e) => e,
            None => {
                ui.label(RichText::new(format!("(export #{one_based} out of range)")).weak());
                return;
            }
        };
        let class = pkg.pak.get_class_name(e.class_index);
        let name = pkg.pak.fname_to_string(&e.object_name);
        let full = pkg.pak.get_export_full_name(one_based);

        ScrollArea::vertical()
            .auto_shrink([false; 2])
            .show(ui, |ui| {
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(class_glyph(&class))
                            .color(class_color(&class))
                            .monospace()
                            .size(18.0),
                    );
                    ui.heading(RichText::new(&name).color(class_color(&class)));
                    ui.label(RichText::new(format!("· {}", class)).weak());
                });
                ui.label(RichText::new(&full).monospace().small().weak());
                ui.add_space(6.0);

                let outer = if e.outer_index > 0 {
                    pkg.pak.get_export_full_name(e.outer_index)
                } else if e.outer_index < 0 {
                    pkg.pak.get_import_full_name(e.outer_index)
                } else {
                    "<root>".to_string()
                };
                let super_s = if e.super_index > 0 {
                    pkg.pak.get_export_full_name(e.super_index)
                } else if e.super_index < 0 {
                    pkg.pak.get_import_full_name(e.super_index)
                } else {
                    "<none>".to_string()
                };
                let archetype = if e.archetype > 0 {
                    pkg.pak.get_export_full_name(e.archetype)
                } else if e.archetype < 0 {
                    pkg.pak.get_import_full_name(e.archetype)
                } else {
                    "<none>".to_string()
                };

                let rows: Vec<(&str, String)> = vec![
                    ("index", format!("#{one_based}")),
                    ("class", format!("{} (raw {})", class, e.class_index)),
                    ("super", format!("{} (raw {})", super_s, e.super_index)),
                    ("outer", format!("{} (raw {})", outer, e.outer_index)),
                    ("archetype", format!("{} (raw {})", archetype, e.archetype)),
                    ("object_flags", format!("0x{:016x}", e.object_flags)),
                    ("export_flags", format!("0x{:08x}", e.export_flags)),
                    ("serial_offset", format!("0x{:x}", e.serial_offset)),
                    ("serial_size", format!("{} bytes", e.serial_size)),
                    ("package_flags", format!("0x{:08x}", e.package_flags)),
                ];

                egui::CollapsingHeader::new(RichText::new("Metadata").strong())
                    .default_open(true)
                    .show(ui, |ui| {
                        TableBuilder::new(ui)
                            .striped(true)
                            .resizable(true)
                            .column(Column::auto().at_least(160.0))
                            .column(Column::remainder())
                            .body(|mut body| {
                                for (k, v) in &rows {
                                    body.row(20.0, |mut row| {
                                        row.col(|ui| {
                                            ui.label(
                                                RichText::new(*k)
                                                    .monospace()
                                                    .color(Color32::from_rgb(0x9c, 0xdc, 0xfe)),
                                            );
                                        });
                                        row.col(|ui| {
                                            ui.label(RichText::new(v).monospace());
                                        });
                                    });
                                }
                            });
                    });

                egui::CollapsingHeader::new(RichText::new("Schema").strong())
                    .default_open(true)
                    .show(ui, |ui| {
                        self.view_export_schema(ui, pkg, e, &class);
                    });

                egui::CollapsingHeader::new(RichText::new("Hex dump").strong())
                    .default_open(false)
                    .show(ui, |ui| {
                        self.view_export_hex(ui, pkg, e);
                    });

                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if e.outer_index > 0 && ui.button("↗ open outer").clicked() {
                        let title = pkg.pak.fname_to_string(
                            &pkg.pak.export_table[(e.outer_index - 1) as usize].object_name,
                        );
                        actions.push(Action::OpenTab(Tab {
                            pkg: pi,
                            kind: TabKind::Export(e.outer_index),
                            title,
                        }));
                    }
                    if e.outer_index < 0 && ui.button("↗ open outer (import)").clicked() {
                        let n = pkg.pak.fname_to_string(
                            &pkg.pak.import_table[((-e.outer_index) - 1) as usize].object_name,
                        );
                        actions.push(Action::OpenTab(Tab {
                            pkg: pi,
                            kind: TabKind::Import(-e.outer_index),
                            title: format!("⤓ {n}"),
                        }));
                    }
                    if e.super_index > 0 && ui.button("↗ open super").clicked() {
                        let title = pkg.pak.fname_to_string(
                            &pkg.pak.export_table[(e.super_index - 1) as usize].object_name,
                        );
                        actions.push(Action::OpenTab(Tab {
                            pkg: pi,
                            kind: TabKind::Export(e.super_index),
                            title,
                        }));
                    }
                });
            });
    }

    fn view_export_schema(&self, ui: &mut Ui, pkg: &LoadedUpk, e: &upkreader::Export, class: &str) {
        use crate::schema::{SchemaParseCtx, parse_export_schema};

        let off = e.serial_offset as usize;
        let sz = e.serial_size as usize;
        if off + sz > pkg.bytes.len() {
            ui.label(RichText::new("(serial range out of bounds)").weak());
            return;
        }
        let blob = &pkg.bytes[off..off + sz];
        let ctx = SchemaParseCtx {
            p_ver: pkg.header.p_ver,
            cooked_for_console: false,
        };

        match parse_export_schema(blob, class, &pkg.pak, ctx) {
            Ok(Some(entry)) => {
                ui.label(RichText::new(summarize_schema(&entry)).monospace());
                ui.add_space(2.0);
                ui.label(
                    RichText::new(format!("{entry:#?}"))
                        .monospace()
                        .small()
                        .color(Color32::from_rgb(0xb0, 0xb0, 0xb0)),
                );
            }
            Ok(None) => {
                ui.label(
                    RichText::new(format!(
                        "Not a meta-class ({}). Schema parsing is reserved for UField/UStruct/UProperty/UFunction/UEnum/UScriptStruct.",
                        class
                    ))
                    .weak(),
                );
            }
            Err(e) => {
                ui.colored_label(
                    Color32::from_rgb(0xff, 0xa0, 0xa0),
                    format!("schema parse error: {e}"),
                );
            }
        }
    }

    fn view_export_hex(&self, ui: &mut Ui, pkg: &LoadedUpk, e: &upkreader::Export) {
        let off = e.serial_offset as usize;
        let sz = e.serial_size as usize;
        if off + sz > pkg.bytes.len() {
            ui.label(RichText::new("(serial range out of bounds)").weak());
            return;
        }
        let data = &pkg.bytes[off..off + sz];
        hex_view(ui, data, off as u64);
    }

    fn view_import(
        &self,
        ui: &mut Ui,
        pkg: &LoadedUpk,
        pi: usize,
        one_based: i32,
        actions: &mut Vec<Action>,
    ) {
        let idx = (one_based - 1) as usize;
        let imp = match pkg.pak.import_table.get(idx) {
            Some(i) => i,
            None => {
                ui.label(RichText::new(format!("(import #{one_based} out of range)")).weak());
                return;
            }
        };
        let name = pkg.pak.fname_to_string(&imp.object_name);
        let cls = pkg.pak.fname_to_string(&imp.class_name);
        let cls_pkg = pkg.pak.fname_to_string(&imp.class_package);
        let path = pkg.pak.get_import_path_name(-one_based);
        let full = pkg.pak.get_import_full_name(-one_based);

        ScrollArea::vertical()
            .auto_shrink([false; 2])
            .show(ui, |ui| {
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new("⤓")
                            .color(Color32::from_rgb(0xff, 0xa5, 0x00))
                            .size(18.0),
                    );
                    ui.heading(RichText::new(&name).color(class_color(&cls)));
                    ui.label(RichText::new(format!("· {cls}")).weak());
                });
                ui.label(RichText::new(&full).monospace().small().weak());
                ui.add_space(6.0);

                let rows: Vec<(&str, String)> = vec![
                    ("index", format!("-{one_based}")),
                    ("class_package", cls_pkg.clone()),
                    ("class_name", cls.clone()),
                    ("outer_index", imp.outer_index.to_string()),
                    ("object_name", name.clone()),
                    ("resolved path", path.clone()),
                ];
                TableBuilder::new(ui)
                    .striped(true)
                    .resizable(true)
                    .column(Column::auto().at_least(160.0))
                    .column(Column::remainder())
                    .body(|mut body| {
                        for (k, v) in &rows {
                            body.row(20.0, |mut row| {
                                row.col(|ui| {
                                    ui.label(
                                        RichText::new(*k)
                                            .monospace()
                                            .color(Color32::from_rgb(0x9c, 0xdc, 0xfe)),
                                    );
                                });
                                row.col(|ui| {
                                    ui.label(RichText::new(v).monospace());
                                });
                            });
                        }
                    });

                ui.add_space(8.0);
                if imp.outer_index < 0 && ui.button("↗ open outer (import)").clicked() {
                    actions.push(Action::OpenTab(Tab {
                        pkg: pi,
                        kind: TabKind::Import(-imp.outer_index),
                        title: format!(
                            "⤓ {}",
                            pkg.pak.fname_to_string(
                                &pkg.pak.import_table[((-imp.outer_index) - 1) as usize]
                                    .object_name,
                            )
                        ),
                    }));
                }
                if imp.outer_index > 0 && ui.button("↗ open outer (export)").clicked() {
                    actions.push(Action::OpenTab(Tab {
                        pkg: pi,
                        kind: TabKind::Export(imp.outer_index),
                        title: pkg.pak.fname_to_string(
                            &pkg.pak.export_table[(imp.outer_index - 1) as usize].object_name,
                        ),
                    }));
                }
            });
    }
}

fn hex_view(ui: &mut Ui, data: &[u8], base_off: u64) {
    let row_h = 16.0;
    let total_rows = data.len().div_ceil(16);
    ui.label(
        RichText::new(format!("{} bytes  ·  base 0x{:x}", data.len(), base_off))
            .weak()
            .small(),
    );
    ui.separator();

    egui::Frame::NONE
        .fill(Color32::from_rgb(0x14, 0x14, 0x14))
        .inner_margin(egui::Margin::same(6))
        .show(ui, |ui| {
            ScrollArea::vertical()
                .max_height(420.0)
                .auto_shrink([false; 2])
                .show_rows(ui, row_h, total_rows, |ui, row_range| {
                    for r in row_range {
                        let row_off = r * 16;
                        let end = (row_off + 16).min(data.len());
                        let bytes = &data[row_off..end];

                        let mut hex_part = String::with_capacity(16 * 3);
                        for (i, b) in bytes.iter().enumerate() {
                            if i == 8 {
                                hex_part.push(' ');
                            }
                            hex_part.push_str(&format!("{:02x} ", b));
                        }
                        for _ in bytes.len()..16 {
                            hex_part.push_str("   ");
                        }
                        let ascii: String = bytes
                            .iter()
                            .map(|&b| {
                                if (0x20..0x7f).contains(&b) {
                                    b as char
                                } else {
                                    '.'
                                }
                            })
                            .collect();

                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new(format!("{:08x}", base_off + row_off as u64))
                                    .monospace()
                                    .color(Color32::from_rgb(0x8a, 0xb4, 0xf8)),
                            );
                            ui.label(
                                RichText::new(hex_part)
                                    .monospace()
                                    .color(Color32::from_rgb(0xdc, 0xdc, 0xdc)),
                            );
                            ui.label(
                                RichText::new(ascii)
                                    .monospace()
                                    .color(Color32::from_rgb(0xa8, 0xc7, 0xa0)),
                            );
                        });
                    }
                });
        });
}

fn summarize_schema(e: &crate::schema::SchemaEntry) -> String {
    use crate::schema::SchemaEntry::*;
    match e {
        Struct { header } => format!(
            "Struct  super={}  children=0x{:x}",
            header.super_struct, header.children
        ),
        Function { header, extra } => format!(
            "Function  super={}  children=0x{:x}  flags=0x{:08x}  iNative={}",
            header.super_struct, header.children, extra.function_flags, extra.i_native
        ),
        State { header, extra } => format!(
            "State  super={}  children=0x{:x}  state_flags=0x{:08x}  funcs={}",
            header.super_struct,
            header.children,
            extra.state_flags,
            extra.func_map.len()
        ),
        Class { header, extra, .. } => format!(
            "Class  super={}  children=0x{:x}  class_flags=0x{:08x}  CDO=#{}  ifaces={}",
            header.super_struct,
            header.children,
            extra.class_flags,
            extra.class_default_object,
            extra.interfaces.len()
        ),
        ScriptStruct { header, extra } => format!(
            "ScriptStruct  super={}  children=0x{:x}  struct_flags=0x{:08x}",
            header.super_struct, header.children, extra.struct_flags
        ),
        Enum { names, .. } => format!("Enum  [{}]", names.len()),
        Const { value, .. } => format!("Const = {value:?}"),
        Property(p) => {
            let c = p.common();
            format!(
                "{:?}  dim={}  flags=0x{:016x}",
                std::mem::discriminant(p),
                c.array_dim,
                c.property_flags
            )
        }
        OpaqueChild { class_name, next } => {
            format!("OpaqueChild({class_name})  next={next}")
        }
    }
}
