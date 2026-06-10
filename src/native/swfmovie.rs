use std::{
    fs::File,
    io::{Result, Write},
    path::{Path, PathBuf},
};

use crate::{
    native::{NativePayload, NativeRead, NativeReadCtx, NativeSerializer},
    upkprops::PropertyValue,
};

#[derive(Debug, Clone)]
pub struct SwfMoviePayload {
    pub raw_data: Vec<u8>,
    pub recovered_via_schema: bool,
}

pub struct SwfMovieSer;

impl NativeSerializer for SwfMovieSer {
    fn class_name(&self) -> &'static str {
        "SwfMovie"
    }

    fn read(&self, ctx: &NativeReadCtx) -> Result<NativeRead> {
        let (raw_data, via_schema) = ctx
            .props
            .iter()
            .find(|p| p.name == "RawData")
            .map(|p| match &p.value {
                PropertyValue::Array(arr) => {
                    let bytes: Vec<u8> = arr
                        .iter()
                        .filter_map(|el| match el {
                            PropertyValue::Byte(b) => Some(*b),
                            _ => None,
                        })
                        .collect();
                    (bytes, true)
                }
                PropertyValue::Raw(buf) => (buf.clone(), false),
                _ => (Vec::new(), false),
            })
            .unwrap_or((Vec::new(), false));

        let payload = SwfMoviePayload {
            raw_data,
            recovered_via_schema: via_schema,
        };

        let consumed = if payload.raw_data.is_empty() {
            Vec::new()
        } else {
            vec!["RawData".to_string()]
        };
        let payload = NativePayload::SwfMovie(payload);
        Ok(NativeRead {
            payload,
            consumed_props: consumed,
        })
    }

    fn emit_external(
        &self,
        payload: &NativePayload,
        dir: &Path,
        stem: &str,
    ) -> Result<Vec<PathBuf>> {
        let p = match payload {
            NativePayload::SwfMovie(p) => p,
            _ => return Ok(Vec::new()),
        };
        if p.raw_data.is_empty() {
            eprintln!(
                "  \x1b[33mgfx\x1b[0m: {stem} has no RawData payload — \
                 check that the SwfMovie/GFxMovieInfo export actually carries Flash bytes"
            );
            return Ok(Vec::new());
        }

        let head = &p.raw_data[..p.raw_data.len().min(4)];
        if !(head.starts_with(b"GFX")
            || head.starts_with(b"CWS")
            || head.starts_with(b"FWS")
            || head.starts_with(b"ZWS"))
        {
            eprintln!(
                "  \x1b[33mgfx\x1b[0m: {stem} RawData magic 0x{:02x?} does not look like Flash; \
                 writing anyway",
                head
            );
        }

        let gfx_path = dir.join(format!("{stem}.gfx"));
        File::create(&gfx_path)?.write_all(&p.raw_data)?;
        println!(
            "  \x1b[36mgfx\x1b[0m → \x1b[32m{}\x1b[0m  ({} bytes)",
            gfx_path.display(),
            p.raw_data.len()
        );
        Ok(vec![gfx_path])
    }
}
