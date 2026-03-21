use std::{
    collections::HashMap,
    fs, io,
    path::{Path, PathBuf},
    sync::OnceLock,
};

use crate::{exe_dir, patch_fmt, upk};
use crate::mod_toml_mini;

static PATCH_MAP: OnceLock<HashMap<String, Vec<patch_fmt::CdoPatch>>> = OnceLock::new();
static CACHE: OnceLock<std::sync::Mutex<HashMap<String, PathBuf>>> = OnceLock::new();

fn patch_map() -> &'static HashMap<String, Vec<patch_fmt::CdoPatch>> {
    PATCH_MAP.get_or_init(|| unsafe { load_mods_dir() })
}
unsafe fn load_mods_dir() -> HashMap<String, Vec<patch_fmt::CdoPatch>> {
    let mut map: HashMap<String, Vec<patch_fmt::CdoPatch>> = HashMap::new();

    let Some(exe) = exe_dir() else { return map };
    let mods_dir = PathBuf::from(&exe).join("Mods");
    let Ok(rd) = fs::read_dir(&mods_dir) else {
        return map;
    };

    for entry in rd.flatten() {
        let path = entry.path();

        if path.is_dir() {
            let toml_path = path.join("mod.toml");
            if toml_path.exists() {
                load_mod_toml(&path, &toml_path, &mut map);
            }
        } else {
            let fname = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            let pkg = if let Some(r) = fname.strip_prefix("ScriptPatch_") {
                r.strip_suffix(".bin").unwrap_or("")
            } else {
                continue;
            };
            if pkg.is_empty() {
                continue;
            }
            let Ok(data) = fs::read(&path) else { continue };
            if let Ok(patches) = patch_fmt::load_patch_bin(&data) {
                map.entry(pkg.to_ascii_lowercase())
                    .or_default()
                    .extend(patches);
            }
        }
    }

    map
}

fn load_mod_toml(
    mod_dir: &Path,
    toml_path: &Path,
    map: &mut HashMap<String, Vec<patch_fmt::CdoPatch>>,
) {
    let Ok(text) = fs::read_to_string(toml_path) else {
        return;
    };
    let patches = mod_toml_mini::parse(&text, mod_dir);
    for (pkg, cdo_patches) in patches {
        map.entry(pkg).or_default().extend(cdo_patches);
    }
}

pub(super) fn get_patched_path(original_path: &str) -> Option<PathBuf> {
    let stem = Path::new(original_path)
        .file_stem()?
        .to_str()?
        .to_ascii_lowercase();

    let patches = patch_map().get(&stem)?;
    if patches.is_empty() {
        return None;
    }

    let cache = CACHE.get_or_init(|| std::sync::Mutex::new(HashMap::new()));
    let key = original_path.to_ascii_lowercase();

    {
        let c = cache.lock().ok()?;
        if let Some(p) = c.get(&key) {
            if p.exists() {
                return Some(p.clone());
            }
        }
    }

    let raw = fs::read(original_path).ok()?;
    let patched = upk::apply_cdo_patches(&raw, patches).ok()?;

    let tmp_dir = temp_dir();
    fs::create_dir_all(&tmp_dir).ok()?;
    let out_path = tmp_dir.join(format!("{}_{:016x}.upk", stem, fnv1a(&raw)));
    fs::write(&out_path, &patched).ok()?;

    let mut c = cache.lock().ok()?;
    c.insert(key, out_path.clone());
    Some(out_path)
}

fn temp_dir() -> PathBuf {
    std::env::temp_dir().join("ue3mods")
}

fn fnv1a(data: &[u8]) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    for &b in data {
        h ^= b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01B3);
    }
    h
}
