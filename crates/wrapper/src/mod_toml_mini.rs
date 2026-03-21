use super::patch_fmt::CdoPatch;
use std::{collections::HashMap, fs, path::Path};

pub(super) fn parse(text: &str, mod_dir: &Path) -> HashMap<String, Vec<CdoPatch>> {
    let mut out: HashMap<String, Vec<CdoPatch>> = HashMap::new();
    let mut cur_package = String::new();
    let mut cur_dir = String::new();
    let mut cur_orig = String::new();

    for raw_line in text.lines() {
        let line = raw_line.trim();

        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        if line == "[[patch]]" {
            cur_package.clear();
            cur_dir.clear();
            cur_orig.clear();
            continue;
        }

        if line == "[[patch.replace]]" {
            cur_orig.clear();
            continue;
        }

        if let Some((k, v)) = split_kv(line) {
            match k {
                "package" => cur_package = v.to_owned(),
                "dir" => cur_dir = v.to_owned(),
                "original" => cur_orig = v.to_owned(),
                "modfile" => {
                    if cur_package.is_empty() || cur_dir.is_empty() || cur_orig.is_empty() {
                        continue;
                    }
                    let blob_path = find_blob(mod_dir, &cur_dir, v);
                    let Ok(data) = fs::read(&blob_path) else {
                        continue;
                    };
                    let patch = CdoPatch {
                        object_path: cur_orig.clone(),
                        data,
                    };
                    out.entry(cur_package.to_ascii_lowercase())
                        .or_default()
                        .push(patch);
                }
                _ => {}
            }
        }
    }

    out
}

fn split_kv(line: &str) -> Option<(&str, &str)> {
    let (k, rest) = line.split_once('=')?;
    let k = k.trim();
    let v = rest.trim().trim_matches('"');
    Some((k, v))
}

fn find_blob(mod_dir: &Path, subdir: &str, stem: &str) -> std::path::PathBuf {
    let dir = mod_dir.join(subdir);
    if let Ok(rd) = std::fs::read_dir(&dir) {
        for entry in rd.flatten() {
            let p = entry.path();
            if p.file_stem().and_then(|s| s.to_str()) == Some(stem) {
                return p;
            }
        }
    }
    dir.join(stem)
}
