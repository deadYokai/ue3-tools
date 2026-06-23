use std::{
    collections::HashMap,
    io::{Read, Result, Seek},
    path::{Path, PathBuf},
    rc::Rc,
};

use crate::{
    schemadb::{ResolvedRef, SchemaDb},
    upkprops::Property,
    upkreader::UPKPak,
    versions::BULKDATA_STORE_IN_SEPARATE_FILE,
};
use byteorder::{LittleEndian, ReadBytesExt};

pub mod soundnodewave;
pub mod swfmovie;
pub mod texture2d;

pub use soundnodewave::{SoundNodeWavePayload, SoundNodeWaveSer};
pub use swfmovie::{SwfMoviePayload, SwfMovieSer};
pub use texture2d::{Mip, MipSource, Texture2DPayload, Texture2DSer};

#[derive(Debug, Clone, Default)]
pub struct BulkBlock {
    pub flags: u32,
    pub element_count: i32,
    pub size_on_disk: i32,
    pub offset_in_file: i32,
    pub data: Vec<u8>,
}

impl BulkBlock {
    pub fn read<R: Read + Seek>(r: &mut R) -> std::io::Result<Self> {
        let flags = r.read_u32::<LittleEndian>()?;
        let element_count = r.read_i32::<LittleEndian>()?;
        let size_on_disk = r.read_i32::<LittleEndian>()?;
        let offset_in_file = r.read_i32::<LittleEndian>()?;

        let inline = flags & BULKDATA_STORE_IN_SEPARATE_FILE == 0;
        let data = if inline && size_on_disk > 0 {
            let mut buf = vec![0u8; size_on_disk as usize];
            r.read_exact(&mut buf)?;
            buf
        } else {
            Vec::new()
        };
        Ok(Self {
            flags,
            element_count,
            size_on_disk,
            offset_in_file,
            data,
        })
    }

    pub fn is_empty(&self) -> bool {
        self.size_on_disk == 0 && self.element_count == 0
    }

    pub fn is_external(&self) -> bool {
        self.flags & BULKDATA_STORE_IN_SEPARATE_FILE != 0
    }
}

#[derive(Debug, Clone)]
pub enum NativePayload {
    Empty { tail: Vec<u8> },

    Raw { bytes: Vec<u8> },

    NativeProps { fields: Vec<Property> },
    Texture2D(Texture2DPayload),
    SwfMovie(SwfMoviePayload),
    SoundNodeWave(SoundNodeWavePayload),
}

impl NativePayload {
    pub fn variant_label(&self) -> &'static str {
        match self {
            NativePayload::Empty { .. } => "Empty",
            NativePayload::Raw { .. } => "Raw",
            NativePayload::Texture2D(_) => "Texture2D",
            NativePayload::SwfMovie(_) => "SwfMovie",
            NativePayload::SoundNodeWave(_) => "SoundNodeWave",
            NativePayload::NativeProps { .. } => "NativeProps",
        }
    }
}

pub struct NativeRead {
    pub payload: NativePayload,
    pub consumed_props: Vec<String>,
}

impl NativeRead {
    pub fn just(payload: NativePayload) -> Self {
        Self {
            payload,
            consumed_props: Vec::new(),
        }
    }
}

pub struct NativeReadCtx<'a> {
    pub blob: &'a [u8],
    pub props: &'a [Property],
    pub ver: i16,
    pub l_ver: i16,
    pub pak: &'a UPKPak,
    pub db: Option<&'a SchemaDb>,
    pub self_ref: Option<ResolvedRef>,
    pub class_ref: Option<ResolvedRef>,
}

pub trait NativeSerializer {
    fn class_name(&self) -> &'static str;

    fn read(&self, ctx: &NativeReadCtx) -> Result<NativeRead>;

    fn emit_external(
        &self,
        _payload: &NativePayload,
        _dir: &Path,
        _stem: &str,
    ) -> Result<Vec<PathBuf>> {
        Ok(Vec::new())
    }
}

pub struct NativeRegistry {
    map: HashMap<&'static str, Rc<dyn NativeSerializer>>,
}

impl NativeRegistry {
    pub fn empty() -> Self {
        Self {
            map: HashMap::new(),
        }
    }

    pub fn standard() -> Self {
        let mut r = Self::empty();
        r.register(Rc::new(Texture2DSer));
        r.register(Rc::new(SwfMovieSer));
        r.map.insert("GFxMovieInfo", Rc::new(SwfMovieSer));
        r.register(Rc::new(SoundNodeWaveSer));
        r
    }

    pub fn register(&mut self, s: Rc<dyn NativeSerializer>) {
        self.map.insert(s.class_name(), s);
    }

    pub fn for_class(
        &self,
        db: Option<&SchemaDb>,
        class_ref: Option<&ResolvedRef>,
        fallback_class_name: &str,
    ) -> Option<Rc<dyn NativeSerializer>> {
        if let (Some(db), Some(cref)) = (db, class_ref) {
            if let Ok(chain) = db.class_chain(cref) {
                for link in &chain {
                    if let Some(name) = db.export_object_name(link) {
                        if let Some(s) = self.map.get(name.as_str()) {
                            return Some(s.clone());
                        }
                    }
                }
            }
        }
        self.map.get(fallback_class_name).cloned()
    }
}
