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
}

pub struct SwfMovieSer;

impl NativeSerializer for SwfMovieSer {
    fn class_name(&self) -> &'static str {
        "SwfMovie"
    }

    fn read(&self, ctx: &NativeReadCtx) -> Result<NativeRead> {
        let bytes: Vec<u8> = ctx
            .props
            .iter()
            .find(|p| p.name == "RawData")
            .and_then(|p| match &p.value {
                PropertyValue::Array(arr) => Some(
                    arr.iter()
                        .filter_map(|el| match el {
                            PropertyValue::Byte(b) => Some(*b),
                            _ => None,
                        })
                        .collect(),
                ),
                _ => None,
            })
            .unwrap_or_default();

        let payload = NativePayload::SwfMovie(SwfMoviePayload { raw_data: bytes });
        let consumed = if !ctx.blob.is_empty()
            || matches!(
                &payload, NativePayload::SwfMovie(p) if !p.raw_data.is_empty()
            ) {
            vec!["RawData".to_string()]
        } else {
            Vec::new()
        };
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
            return Ok(Vec::new());
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
