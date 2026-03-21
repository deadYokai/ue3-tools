use std::{
    collections::HashMap,
    fs::{self, File},
    io::{BufWriter, Result, Write},
    path::Path,
};

use byteorder::{LittleEndian, WriteBytesExt};

use crate::{
    scriptpatcher::{LinkerPatchData, PatchData, compress_patch},
    upkreader::{UPKPak, UpkHeader},
};

const VER_BYTEPROP_SERIALIZE_ENUM: i16 = 633;
const VER_PROPERTYTAG_BOOL_OPT: i16 = 673;
const VER_HAS_GUID_OFFSETS: i16 = 623;
const VER_HAS_THUMBNAIL: i16 = 584;
const VER_HAS_EXTRA_PKGS: i16 = 516;
const VER_HAS_TEX_ALLOCS: i16 = 767;

pub struct FontConfig {
    pub font_path: String,
    pub font_name: String,
    pub size_pt: f32,
    pub dpi: u32,
    pub tex_width: u32,
    pub tex_height: u32,
    pub x_pad: i32,
    pub y_pad: i32,
    pub chars: Option<String>,
    pub upk_version: i16,
}

impl Default for FontConfig {
    fn default() -> Self {
        Self {
            font_path: String::new(),
            font_name: "MyFont".into(),
            size_pt: 16.0,
            dpi: 72,
            tex_width: 512,
            tex_height: 512,
            x_pad: 1,
            y_pad: 1,
            chars: None,
            upk_version: 684,
        }
    }
}

pub struct FChar {
    start_u: i32,
    start_v: i32,
    u_size: i32,
    v_size: i32,
    tex_idx: u8,
    vert_off: i32,
}

pub struct TexPage {
    pub rgba: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

pub struct Raster {
    pub fchars: Vec<FChar>,
    pub pages: Vec<TexPage>,
    pub em_scale: f32,
    pub ascent: f32,
    pub descent: f32,
    pub leading: f32,
}

pub fn create_font_upk(cfg: &FontConfig, out_path: &Path) -> Result<()> {
    let r = rasterize(cfg)?;
    let pkg = &cfg.font_name;
    let ver = cfg.upk_version;

    let mut nt = build_name_table(pkg, &r.pages, ver);

    let page_names: Vec<String> = (0..r.pages.len()).map(|i| page_name(pkg, i)).collect();
    for n in &page_names {
        nt.add(n);
    }

    let imports = build_imports(&nt);

    let num_exports = 1 + r.pages.len();
    let tex_refs: Vec<i32> = (2..=r.pages.len() as i32 + 1).collect();

    let font_data = serial_font(
        &r.fchars, &tex_refs, r.em_scale, r.ascent, r.descent, r.leading, &nt, ver,
    );
    let tex_data: Vec<Vec<u8>> = r
        .pages
        .iter()
        .enumerate()
        .map(|(i, p)| serial_texture2d(p, &page_names[i], &nt, ver))
        .collect();

    let h = header_binary_size(ver);
    let n = nt.byte_size();
    let e = num_exports * EXPORT_ENTRY_SIZE;
    let imp = imports.len() * IMPORT_ENTRY_SIZE;
    let d = num_exports * 4;

    let name_off = h as i32;
    let export_off = (h + n) as i32;
    let import_off = (h + n + e) as i32;
    let depend_off = (h + n + e + imp) as i32;
    let guid_off = (h + n + e + imp + d) as i32;

    let serial_start = h + n + e + imp + d;
    let header_size = serial_start as i32;

    let font_offset = serial_start as i32;
    let font_size = font_data.len() as i32;

    let mut tex_offsets = Vec::with_capacity(r.pages.len());
    let mut cur = serial_start + font_data.len();
    for td in &tex_data {
        tex_offsets.push(cur as i32);
        cur += td.len();
    }

    let file = File::create(out_path)?;
    let mut w = BufWriter::new(file);

    write_upk_header(
        &mut w,
        ver,
        header_size,
        name_off,
        export_off,
        import_off,
        depend_off,
        guid_off,
        nt.names.len(),
        num_exports,
        imports.len(),
    )?;

    nt.write(&mut w)?;

    write_export(
        &mut w,
        ver,
        -2,
        0,
        0,
        nt.idx(pkg),
        0,
        0,
        0x0000_0000_0000_000C,
        font_size,
        font_offset,
    )?;
    for (i, td) in tex_data.iter().enumerate() {
        write_export(
            &mut w,
            ver,
            -3,
            0,
            1,
            nt.idx(&page_names[i]),
            0,
            0,
            0x0000_0000_0000_0004,
            td.len() as i32,
            tex_offsets[i],
        )?;
    }

    for imp in &imports {
        imp.write(&mut w)?;
    }
    for _ in 0..num_exports {
        w.write_i32::<LittleEndian>(0)?;
    }

    w.write_all(&font_data)?;
    for td in &tex_data {
        w.write_all(td)?;
    }

    println!(
        "Wrote font UPK: {}  ({} chars, {} page(s))",
        out_path.display(),
        r.fchars.len(),
        r.pages.len()
    );
    Ok(())
}
pub fn create_font_patch(
    upk_raw: &[u8],
    header: &UpkHeader,
    pak: &UPKPak,
    font_object_name: &str,
    cfg: &FontConfig,
    out_dir: &Path,
) -> Result<()> {
    let ver = header.p_ver;

    let needle = font_object_name.to_lowercase();
    let font_exp_idx = pak
        .export_table
        .iter()
        .enumerate()
        .find(|(i, _)| {
            let class = pak.get_class_name(pak.export_table[*i].class_index);
            let name = pak.fname_to_string(&pak.export_table[*i].object_name);
            class == "Font" && name.to_lowercase() == needle
        })
        .map(|(i, _)| i)
        .ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("No Font export named '{}' found in UPK", font_object_name),
            )
        })?;

    let font_exp = &pak.export_table[font_exp_idx];
    let font_name = pak.fname_to_string(&font_exp.object_name);
    let font_path_name = pak.get_export_path_name((font_exp_idx + 1) as i32);

    println!(
        "Found font: {} (export #{})",
        font_path_name,
        font_exp_idx + 1
    );

    let font_1based = (font_exp_idx + 1) as i32;
    let mut tex_exports: Vec<usize> = pak
        .export_table
        .iter()
        .enumerate()
        .filter(|(i, e)| {
            e.outer_index == font_1based && pak.get_class_name(e.class_index) == "Texture2D"
        })
        .map(|(i, _)| i)
        .collect();

    tex_exports.sort_by_key(|&i| pak.export_table[i].serial_offset);

    println!(
        "Found {} texture page(s) for '{}'",
        tex_exports.len(),
        font_name
    );

    let r = rasterize(cfg)?;

    if r.pages.len() != tex_exports.len() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!(
                "Original font '{}' has {} texture page(s) but the new rasterization \
                 produced {} page(s). CDO patches cannot add/remove exports.\n\
                 Hint: adjust --tex-width / --tex-height or --size so the glyph packing \
                 results in exactly {} page(s).",
                font_name,
                tex_exports.len(),
                r.pages.len(),
                tex_exports.len(),
            ),
        ));
    }

    let tex_refs: Vec<i32> = tex_exports.iter().map(|&i| (i + 1) as i32).collect();

    let mut nt = build_name_table(&font_name, &r.pages, ver);
    let page_names: Vec<String> = tex_exports
        .iter()
        .map(|&i| pak.fname_to_string(&pak.export_table[i].object_name))
        .collect();
    for n in &page_names {
        nt.add(n);
    }

    let font_serial = serial_font(
        &r.fchars, &tex_refs, r.em_scale, r.ascent, r.descent, r.leading, &nt, ver,
    );

    let pkg_name = {
        let parts: Vec<&str> = font_path_name.split('.').collect();
        if parts.len() > 1 {
            parts[0].to_string()
        } else {
            font_path_name.clone()
        }
    };

    let mut patch = LinkerPatchData::new(pkg_name.clone());

    let font_inner_path = strip_package_prefix(&font_path_name, &pkg_name);
    patch.add_cdo_patch(PatchData::new(font_inner_path.clone(), font_serial));
    println!(
        "  CDO patch: '{}'  ({} bytes)",
        font_inner_path,
        font_serial_len(&r.fchars, tex_refs.len())
    );

    for (page_idx, &tex_exp_i) in tex_exports.iter().enumerate() {
        let tex_path = pak.get_export_path_name((tex_exp_i + 1) as i32);
        let tex_inner = strip_package_prefix(&tex_path, &pkg_name);
        let page = &r.pages[page_idx];
        let tex_serial = serial_texture2d(page, &page_names[page_idx], &nt, ver);

        println!(
            "  CDO patch: '{}'  ({}×{}, {} bytes)",
            tex_inner,
            page.width,
            page.height,
            tex_serial.len()
        );

        patch.add_cdo_patch(PatchData::new(tex_inner, tex_serial));
    }

    std::fs::create_dir_all(out_dir)?;
    let out_path = out_dir.join(format!("ScriptPatch_{}.bin", pkg_name));
    let bin = compress_patch(&patch)?;
    std::fs::write(&out_path, &bin)?;

    println!(
        "Wrote font patch: {}  ({} CDO patch(es))",
        out_path.display(),
        patch.modified_class_default_objects.len()
    );
    Ok(())
}

fn rasterize(cfg: &FontConfig) -> Result<Raster> {
    use freetype::face::LoadFlag;

    let lib = freetype::Library::init()
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))?;
    let face = lib
        .new_face(&cfg.font_path, 0)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))?;

    face.set_char_size(0, (cfg.size_pt * 64.0) as isize, cfg.dpi, cfg.dpi)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))?;

    let metrics = face.clone();
    let px_ascend = (metrics.ascender() >> 6) as i32;
    let px_descend = (metrics.descender() >> 6) as i32; // negative
    let px_height = (metrics.height() >> 6) as i32;

    let px_em = (px_ascend - px_descend).max(1) as f32;
    let em_scale = 1024.0 / px_em;
    let ascent = px_ascend as f32 * em_scale;
    let descent = px_descend as f32 * em_scale;
    let leading = (px_height - px_ascend + px_descend) as f32 * em_scale;

    let rasterise: Vec<u32> = if let Some(ref s) = cfg.chars {
        s.chars().map(|c| c as u32).collect()
    } else {
        (32u32..127).collect()
    };

    struct Glyph {
        code: u32,
        rgba: Vec<u8>,
        bw: i32,
        bh: i32,
        top: i32,
        adv: i32,
    }

    let mut glyphs: Vec<Glyph> = Vec::new();
    for &code in &rasterise {
        if face.load_char(code as usize, LoadFlag::RENDER).is_err() {
            glyphs.push(Glyph {
                code,
                rgba: vec![],
                bw: 0,
                bh: 0,
                top: 0,
                adv: 0,
            });
            continue;
        }
        let g = face.glyph();
        let bm = g.bitmap();
        let bw = bm.width() as i32;
        let bh = bm.rows() as i32;
        let top = g.bitmap_top();
        let adv = (g.advance().x >> 6) as i32;

        if bw <= 0 || bh <= 0 {
            glyphs.push(Glyph {
                code,
                rgba: vec![],
                bw: adv,
                bh: 0,
                top: 0,
                adv,
            });
            continue;
        }

        let buf = bm.buffer();
        let pitch = bm.pitch().unsigned_abs() as usize;
        let mut rgba = vec![0u8; (bw * bh * 4) as usize];
        for y in 0..bh as usize {
            for x in 0..bw as usize {
                let alpha = buf[y * pitch + x];
                let dst = (y * bw as usize + x) * 4;
                rgba[dst] = 255; // B (BGRA = PF_A8R8G8B8)
                rgba[dst + 1] = 255; // G
                rgba[dst + 2] = 255; // R
                rgba[dst + 3] = alpha;
            }
        }
        glyphs.push(Glyph {
            code,
            rgba,
            bw,
            bh,
            top,
            adv,
        });
    }

    let tw = cfg.tex_width as i32;
    let tmh = cfg.tex_height as i32;
    let xpad = cfg.x_pad;
    let ypad = cfg.y_pad;

    let mut fchars: Vec<FChar> = (0u32..256)
        .map(|_| FChar {
            start_u: 0,
            start_v: 0,
            u_size: 0,
            v_size: 0,
            tex_idx: 0,
            vert_off: 0,
        })
        .collect();

    let mut pages: Vec<TexPage> = Vec::new();
    let mut page_buf: Vec<u8> = vec![0u8; (tw * tmh * 4) as usize];
    let mut page_idx: u8 = 0;
    let mut cx: i32 = xpad;
    let mut cy: i32 = ypad;
    let mut row_h: i32 = 0;
    let mut max_used_y: i32 = ypad;

    for g in &glyphs {
        let slot = g.code as usize;
        if slot >= 256 {
            continue;
        }

        let gw = g.bw.max(g.adv).max(1) + xpad;
        let gh = g.bh.max(1) + ypad;

        if cx + gw > tw {
            cx = xpad;
            cy += row_h + ypad;
            row_h = 0;
        }

        if cy + gh > tmh {
            pages.push(flush_page(&page_buf, max_used_y, tw, cfg.tex_height));
            page_buf = vec![0u8; (tw * tmh * 4) as usize];
            page_idx += 1;
            cx = xpad;
            cy = ypad;
            row_h = 0;
            max_used_y = ypad;
        }

        if !g.rgba.is_empty() && g.bw > 0 && g.bh > 0 {
            for py in 0..g.bh {
                for px in 0..g.bw {
                    let src = ((py * g.bw + px) * 4) as usize;
                    let dx = cx + px;
                    let dy = cy + py;
                    if dx < tw && dy < tmh {
                        let dst = ((dy * tw + dx) * 4) as usize;
                        if dst + 3 < page_buf.len() && src + 3 < g.rgba.len() {
                            page_buf[dst] = g.rgba[src];
                            page_buf[dst + 1] = g.rgba[src + 1];
                            page_buf[dst + 2] = g.rgba[src + 2];
                            page_buf[dst + 3] = g.rgba[src + 3];
                        }
                    }
                }
            }
        }

        let vert_off = px_ascend - g.top;
        fchars[slot] = FChar {
            start_u: cx,
            start_v: cy,
            u_size: g.bw.max(g.adv),
            v_size: g.bh,
            tex_idx: page_idx,
            vert_off,
        };

        row_h = row_h.max(gh);
        cx += gw;
        max_used_y = max_used_y.max(cy + row_h);
    }

    pages.push(flush_page(&page_buf, max_used_y, tw, cfg.tex_height));

    Ok(Raster {
        fchars,
        pages,
        em_scale,
        ascent,
        descent,
        leading,
    })
}

fn flush_page(buf: &[u8], max_y: i32, tw: i32, tmh: u32) -> TexPage {
    let actual_h = next_pow2((max_y + 1).max(4) as u32).min(tmh);
    let copy_h = actual_h as i32;
    let n = (tw * copy_h * 4) as usize;
    let mut rgba = vec![0u8; n];
    rgba[..n.min(buf.len())].copy_from_slice(&buf[..n.min(buf.len())]);
    TexPage {
        rgba,
        width: tw as u32,
        height: actual_h,
    }
}

fn serial_font(
    chars: &[FChar],
    tex_refs: &[i32],
    em_scale: f32,
    ascent: f32,
    descent: f32,
    leading: f32,
    nt: &NT,
    ver: i16,
) -> Vec<u8> {
    let mut buf = Vec::new();
    let pw = PW::new(nt, ver);

    let count = chars.len() as i32;
    let arr_size = 4 + count * 21;
    pw.tag(&mut buf, "Characters", "ArrayProperty", arr_size);
    buf.write_i32::<LittleEndian>(count).unwrap();
    for c in chars {
        buf.write_i32::<LittleEndian>(c.start_u).unwrap();
        buf.write_i32::<LittleEndian>(c.start_v).unwrap();
        buf.write_i32::<LittleEndian>(c.u_size).unwrap();
        buf.write_i32::<LittleEndian>(c.v_size).unwrap();
        buf.write_u8(c.tex_idx).unwrap();
        buf.write_i32::<LittleEndian>(c.vert_off).unwrap();
    }

    pw.arr_objs(&mut buf, "Textures", tex_refs);
    pw.int(&mut buf, "Kerning", 0);
    pw.int(&mut buf, "IsRemapped", 0);
    pw.float(&mut buf, "EmScale", em_scale);
    pw.float(&mut buf, "Ascent", ascent);
    pw.float(&mut buf, "Descent", descent);
    pw.float(&mut buf, "Leading", leading);
    pw.float(&mut buf, "ScalingFactor", 1.0);
    pw.none(&mut buf);

    buf.write_i32::<LittleEndian>(0).unwrap();

    buf
}

fn font_serial_len(chars: &[FChar], tex_count: usize) -> usize {
    let arr = 4 + chars.len() * 21;
    let tex = 4 + tex_count * 4;
    let props = 9 * 24 + 9 * 4;
    let none = 8;
    arr + tex + props + none + 4
}

pub fn serial_texture2d(page: &TexPage, _name: &str, nt: &NT, ver: i16) -> Vec<u8> {
    let mut buf = Vec::new();
    let pw = PW::new(nt, ver);

    pw.byte_enum(&mut buf, "Format", "EPixelFormat", "PF_A8R8G8B8");
    pw.byte_enum(&mut buf, "LODGroup", "TextureGroup", "TEXTUREGROUP_UI");
    pw.bool_(&mut buf, "NeverStream", true);
    pw.bool_(&mut buf, "SRGB", false);
    pw.byte_enum(
        &mut buf,
        "CompressionSettings",
        "TextureCompressionSettings",
        "TC_Displacementmap",
    );
    pw.byte_enum(
        &mut buf,
        "MipGenSettings",
        "TextureMipGenSettings",
        "TMGS_NoMipmaps",
    );
    pw.none(&mut buf);

    let px_bytes = page.rgba.len() as i32;
    buf.write_i32::<LittleEndian>(1).unwrap(); // NumMips = 1

    buf.write_u32::<LittleEndian>(0).unwrap(); // BulkDataFlags = inline
    buf.write_i32::<LittleEndian>(px_bytes).unwrap();
    buf.write_i32::<LittleEndian>(px_bytes).unwrap();
    buf.write_i32::<LittleEndian>(-1).unwrap(); // FileOffset placeholder (inline)

    buf.extend_from_slice(&page.rgba);

    buf.write_i32::<LittleEndian>(page.width as i32).unwrap();
    buf.write_i32::<LittleEndian>(page.height as i32).unwrap();

    buf
}

fn header_binary_size(ver: i16) -> usize {
    let mut s: usize = 0;
    s += 4 + 2 + 2; // sign, p_ver, l_ver
    s += 4; // header_size
    s += 4; // path_len (= 0)
    s += 4; // pak_flags
    s += 4 * 7; // name/export/import/depends counts+offsets
    if ver >= VER_HAS_GUID_OFFSETS {
        s += 4 + 4 + 4;
    }
    if ver >= VER_HAS_THUMBNAIL {
        s += 4;
    }
    s += 16; // GUID
    s += 4 + 12; // gen_count(1) + 1 generation
    s += 4 + 4; // engine_ver, cooker_ver
    s += 4 + 4; // compression_method, compressed_chunks_count
    s += 4; // package_source
    if ver >= VER_HAS_EXTRA_PKGS {
        s += 4;
    }
    if ver >= VER_HAS_TEX_ALLOCS {
        s += 4;
    }
    s
}

const EXPORT_ENTRY_SIZE: usize = 68;
const IMPORT_ENTRY_SIZE: usize = 28;

#[allow(clippy::too_many_arguments)]
fn write_export<W: Write>(
    w: &mut W,
    ver: i16,
    class_idx: i32,
    super_idx: i32,
    outer_idx: i32,
    name_idx: i32,
    name_inst: i32,
    archetype: i32,
    flags: u64,
    ser_size: i32,
    ser_off: i32,
) -> Result<()> {
    w.write_i32::<LittleEndian>(class_idx)?;
    w.write_i32::<LittleEndian>(super_idx)?;
    w.write_i32::<LittleEndian>(outer_idx)?;
    w.write_i32::<LittleEndian>(name_idx)?;
    w.write_i32::<LittleEndian>(name_inst)?;
    w.write_i32::<LittleEndian>(archetype)?;
    w.write_u64::<LittleEndian>(flags)?;
    w.write_i32::<LittleEndian>(ser_size)?;
    w.write_i32::<LittleEndian>(ser_off)?;
    w.write_u32::<LittleEndian>(0)?; // export_flags
    w.write_i32::<LittleEndian>(0)?; // generation_net_object_count (count=0)
    for _ in 0..4 {
        w.write_i32::<LittleEndian>(0)?;
    } // package_guid
    w.write_u32::<LittleEndian>(0)?; // package_flags
    let _ = ver;
    Ok(())
}

fn write_upk_header<W: Write>(
    w: &mut W,
    ver: i16,
    header_size: i32,
    name_off: i32,
    export_off: i32,
    import_off: i32,
    depend_off: i32,
    guid_off: i32,
    name_count: usize,
    exp_count: usize,
    imp_count: usize,
) -> Result<()> {
    w.write_u32::<LittleEndian>(0x9E2A83C1)?;
    w.write_i16::<LittleEndian>(ver)?;
    w.write_i16::<LittleEndian>(0)?;
    w.write_i32::<LittleEndian>(header_size)?;
    w.write_i32::<LittleEndian>(0)?; // path_len=0
    w.write_u32::<LittleEndian>(0)?; // pak_flags
    w.write_i32::<LittleEndian>(name_count as i32)?;
    w.write_i32::<LittleEndian>(name_off)?;
    w.write_i32::<LittleEndian>(exp_count as i32)?;
    w.write_i32::<LittleEndian>(export_off)?;
    w.write_i32::<LittleEndian>(imp_count as i32)?;
    w.write_i32::<LittleEndian>(import_off)?;
    w.write_i32::<LittleEndian>(depend_off)?;

    if ver >= VER_HAS_GUID_OFFSETS {
        w.write_i32::<LittleEndian>(guid_off)?;
        w.write_u32::<LittleEndian>(0)?;
        w.write_u32::<LittleEndian>(0)?;
    }
    if ver >= VER_HAS_THUMBNAIL {
        w.write_u32::<LittleEndian>(0)?;
    }
    for seed in [0x12345678u32, 0xDEADBEEF, 0xCAFEBABE, 0xFEEDFACE] {
        w.write_u32::<LittleEndian>(seed)?;
    }
    w.write_i32::<LittleEndian>(1)?;
    w.write_i32::<LittleEndian>(exp_count as i32)?;
    w.write_i32::<LittleEndian>(name_count as i32)?;
    w.write_i32::<LittleEndian>(0)?;
    w.write_i32::<LittleEndian>(12791)?;
    w.write_i32::<LittleEndian>(0)?;
    w.write_u32::<LittleEndian>(0)?;
    w.write_u32::<LittleEndian>(0)?;
    w.write_i32::<LittleEndian>(0)?;
    if ver >= VER_HAS_EXTRA_PKGS {
        w.write_i32::<LittleEndian>(0)?;
    }
    if ver >= VER_HAS_TEX_ALLOCS {
        w.write_i32::<LittleEndian>(0)?;
    }
    Ok(())
}

struct ImportEntry {
    cls_pkg: (i32, i32),
    cls_name: (i32, i32),
    outer: i32,
    obj_name: (i32, i32),
}

impl ImportEntry {
    fn write<W: Write>(&self, w: &mut W) -> Result<()> {
        w.write_i32::<LittleEndian>(self.cls_pkg.0)?;
        w.write_i32::<LittleEndian>(self.cls_pkg.1)?;
        w.write_i32::<LittleEndian>(self.cls_name.0)?;
        w.write_i32::<LittleEndian>(self.cls_name.1)?;
        w.write_i32::<LittleEndian>(self.outer)?;
        w.write_i32::<LittleEndian>(self.obj_name.0)?;
        w.write_i32::<LittleEndian>(self.obj_name.1)?;
        Ok(())
    }
}

fn build_imports(nt: &NT) -> Vec<ImportEntry> {
    vec![
        ImportEntry {
            cls_pkg: (nt.idx("Core"), 0),
            cls_name: (nt.idx("Package"), 0),
            outer: 0,
            obj_name: (nt.idx("Engine"), 0),
        },
        ImportEntry {
            cls_pkg: (nt.idx("Core"), 0),
            cls_name: (nt.idx("Class"), 0),
            outer: -1,
            obj_name: (nt.idx("Font"), 0),
        },
        ImportEntry {
            cls_pkg: (nt.idx("Core"), 0),
            cls_name: (nt.idx("Class"), 0),
            outer: -1,
            obj_name: (nt.idx("Texture2D"), 0),
        },
    ]
}

fn build_name_table(pkg: &str, pages: &[TexPage], _ver: i16) -> NT {
    let mut nt = NT::new();
    nt.add("None");
    nt.add(pkg);
    nt.add("Font");
    nt.add("Texture2D");
    nt.add("Engine");
    nt.add("Core");
    nt.add("Class");
    nt.add("Package");
    nt.add("Characters");
    nt.add("Textures");
    nt.add("Kerning");
    nt.add("IsRemapped");
    nt.add("EmScale");
    nt.add("Ascent");
    nt.add("Descent");
    nt.add("Leading");
    nt.add("ScalingFactor");
    nt.add("Format");
    nt.add("LODGroup");
    nt.add("NeverStream");
    nt.add("SRGB");
    nt.add("CompressionSettings");
    nt.add("MipGenSettings");
    nt.add("IntProperty");
    nt.add("FloatProperty");
    nt.add("BoolProperty");
    nt.add("ByteProperty");
    nt.add("ArrayProperty");
    nt.add("ObjectProperty");
    nt.add("EPixelFormat");
    nt.add("TextureGroup");
    nt.add("TextureCompressionSettings");
    nt.add("TextureMipGenSettings");
    nt.add("PF_A8R8G8B8");
    nt.add("TEXTUREGROUP_UI");
    nt.add("TC_Displacementmap");
    nt.add("TMGS_NoMipmaps");
    let _ = pages;
    nt
}

pub struct NT {
    pub names: Vec<String>,
    map: HashMap<String, i32>,
}

impl NT {
    fn new() -> Self {
        Self {
            names: vec![],
            map: HashMap::new(),
        }
    }

    pub fn add(&mut self, s: &str) -> i32 {
        if let Some(&i) = self.map.get(s) {
            return i;
        }
        let i = self.names.len() as i32;
        self.names.push(s.into());
        self.map.insert(s.into(), i);
        i
    }

    pub fn idx(&self, s: &str) -> i32 {
        *self
            .map
            .get(s)
            .unwrap_or_else(|| panic!("name '{}' missing from NT", s))
    }

    fn write<W: Write>(&self, w: &mut W) -> Result<()> {
        for n in &self.names {
            let b = n.as_bytes();
            w.write_i32::<LittleEndian>((b.len() + 1) as i32)?;
            w.write_all(b)?;
            w.write_u8(0)?;
            w.write_u64::<LittleEndian>(0)?;
        }
        Ok(())
    }

    fn byte_size(&self) -> usize {
        self.names.iter().map(|n| 4 + n.len() + 1 + 8).sum()
    }
}

struct PW<'a> {
    nt: &'a NT,
    ver: i16,
}

impl<'a> PW<'a> {
    fn new(nt: &'a NT, ver: i16) -> Self {
        Self { nt, ver }
    }

    fn fname(&self, buf: &mut Vec<u8>, idx: i32) {
        buf.write_i32::<LittleEndian>(idx).unwrap();
        buf.write_i32::<LittleEndian>(0).unwrap();
    }

    fn tag(&self, buf: &mut Vec<u8>, name: &str, ty: &str, size: i32) {
        self.fname(buf, self.nt.idx(name));
        self.fname(buf, self.nt.idx(ty));
        buf.write_i32::<LittleEndian>(size).unwrap();
        buf.write_i32::<LittleEndian>(0).unwrap();
    }

    fn none(&self, buf: &mut Vec<u8>) {
        self.fname(buf, self.nt.idx("None"));
    }

    fn int(&self, buf: &mut Vec<u8>, name: &str, val: i32) {
        self.tag(buf, name, "IntProperty", 4);
        buf.write_i32::<LittleEndian>(val).unwrap();
    }

    fn float(&self, buf: &mut Vec<u8>, name: &str, val: f32) {
        self.tag(buf, name, "FloatProperty", 4);
        buf.write_f32::<LittleEndian>(val).unwrap();
    }

    fn bool_(&self, buf: &mut Vec<u8>, name: &str, val: bool) {
        if self.ver >= VER_PROPERTYTAG_BOOL_OPT {
            self.tag(buf, name, "BoolProperty", 0);
            buf.write_u8(val as u8).unwrap();
        } else {
            self.tag(buf, name, "BoolProperty", 4);
            buf.write_u32::<LittleEndian>(val as u32).unwrap();
        }
    }

    fn byte_enum(&self, buf: &mut Vec<u8>, name: &str, en_type: &str, en_val: &str) {
        if self.ver >= VER_BYTEPROP_SERIALIZE_ENUM {
            self.tag(buf, name, "ByteProperty", 8);
            self.fname(buf, self.nt.idx(en_type));
            self.fname(buf, self.nt.idx(en_val));
        } else {
            self.tag(buf, name, "ByteProperty", 1);
            buf.write_u8(0).unwrap();
        }
    }

    fn arr_objs(&self, buf: &mut Vec<u8>, name: &str, refs: &[i32]) {
        let cnt = refs.len() as i32;
        let size = 4 + cnt * 4;
        self.tag(buf, name, "ArrayProperty", size);
        buf.write_i32::<LittleEndian>(cnt).unwrap();
        for &r in refs {
            buf.write_i32::<LittleEndian>(r).unwrap();
        }
    }
}

fn next_pow2(mut n: u32) -> u32 {
    if n == 0 {
        return 1;
    }
    n -= 1;
    n |= n >> 1;
    n |= n >> 2;
    n |= n >> 4;
    n |= n >> 8;
    n |= n >> 16;
    n + 1
}

fn page_name(pkg: &str, idx: usize) -> String {
    if idx < 26 {
        format!("{}_Page{}", pkg, (b'A' + idx as u8) as char)
    } else {
        format!(
            "{}_Page{}{}",
            pkg,
            (b'A' + (idx / 26) as u8) as char,
            (b'A' + (idx % 26) as u8) as char
        )
    }
}

fn strip_package_prefix(path: &str, pkg: &str) -> String {
    let prefix = format!("{}.", pkg);
    if path.starts_with(&prefix) {
        path[prefix.len()..].to_string()
    } else {
        path.to_string()
    }
}

pub fn create_font_blobs(cfg: &FontConfig, out_dir: &Path) -> Result<()> {
    let r = rasterize(cfg)?;
    let pkg = &cfg.font_name;
    let ver = cfg.upk_version;

    let mut nt = build_name_table(pkg, &r.pages, ver);
    let page_names: Vec<String> = (0..r.pages.len()).map(|i| page_name(pkg, i)).collect();
    for n in &page_names {
        nt.add(n);
    }

    let tex_refs: Vec<i32> = (2..=r.pages.len() as i32 + 1).collect();

    let font_blob = serial_font(
        &r.fchars, &tex_refs, r.em_scale, r.ascent, r.descent, r.leading, &nt, ver,
    );

    fs::create_dir_all(out_dir)?;

    let font_path = out_dir.join(format!("{}.Font", pkg));
    fs::write(&font_path, &font_blob)?;
    println!(" {} bytes  →  {}", font_blob.len(), font_path.display());

    for (i, page) in r.pages.iter().enumerate() {
        let tex_blob = serial_texture2d(page, &page_names[i], &nt, ver);
        let tex_path = out_dir.join(format!("{}.Texture2D", page_names[i]));
        fs::write(&tex_path, &tex_blob)?;
        println!(
            " {} bytes  →  {}  ({}×{})",
            tex_blob.len(),
            tex_path.display(),
            page.width,
            page.height
        );
    }

    println!(
        "blobs done — {} char(s), {} page(s)",
        r.fchars.iter().filter(|c| c.u_size > 0).count(),
        r.pages.len()
    );
    Ok(())
}
