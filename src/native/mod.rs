use std::{
    collections::HashMap,
    io::Result,
    path::{Path, PathBuf},
    rc::Rc,
};

use crate::{
    schemadb::{ResolvedRef, SchemaDb},
    upkprops::Property,
    upkreader::UPKPak,
};

pub mod swfmovie;
pub mod texture2d;

pub use swfmovie::{SwfMoviePayload, SwfMovieSer};
pub use texture2d::{Mip, MipSource, Texture2DPayload, Texture2DSer};
#[derive(Debug, Clone)]
pub enum NativePayload {
    Empty { tail: Vec<u8> },

    Raw { bytes: Vec<u8> },

    Texture2D(Texture2DPayload),
    SwfMovie(SwfMoviePayload),
}

impl NativePayload {
    pub fn variant_label(&self) -> &'static str {
        match self {
            NativePayload::Empty { .. } => "Empty",
            NativePayload::Raw { .. } => "Raw",
            NativePayload::Texture2D(_) => "Texture2D",
            NativePayload::SwfMovie(_) => "SwfMovie",
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
