use std::{fmt, io::{Error, ErrorKind, Read, Seek}};
use byteorder::{LittleEndian, ReadBytesExt};

pub struct Names
{
    n_len: i32,
    name: Vec<u8>,
    n_fh: i32,
    n_fl: i32
}

pub struct Export
{
    obj_type_ref: i32,
    parent_class_ref: i32,
    owner_ref: i32,
    name_tbl_idx: i32,
    name_count: i32, // if non-zero "_N" added to objName, N = NameCount-1
    field6: i32,
    obj_flags_h: i32,
    obj_flags_l: i32,
    obj_filesize: i32,
    data_offset: i32,
    field11: i32,
    num_additional_fields: i32,
    field13: i32,
    field14: i32,
    field15: i32,
    field16: i32,
    field17: i32,
    unk_fields: Vec<i32>
}

pub struct Import
{
    package_idx: i32,
    unk1: i32,
    obj_type_idx: i32,
    unk2: i32,
    owner_ref: i32,
    name_tbl_idx: i32,
    unk3: i32
}

pub struct GenerationInfo
{
    export_count: i32,
    name_count: i32,
    net_obj_count: i32
}

pub struct UpkHeader
{
    sign: u32,
    p_ver: i16,
    l_ver: i16,
    pub header_size: i32,
    path_len: i32,
    path: Vec<u8>,
    pak_flags: i32,
    name_count: i32,
    name_offset: i32,
    export_count: i32,
    export_offset: i32,
    import_count: i32,
    import_offset: i32,
    depends_offset: i32,
    unk: [i32; 4],
    guid: [i32; 4],
    gen_count: i32,
    gens: Vec<GenerationInfo>,
    engine_ver: i32,
    cooker_ver: i32,
    compression: i32
}

impl fmt::Display for UpkHeader 
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result 
    {
        writeln!(f, "Package Version: {}", self.p_ver)?;
        writeln!(f, "Licensee Version: {}", self.l_ver)?;
        writeln!(f, "Header Size: {}", self.header_size)?;
        writeln!(f, "Folder: {:?}", String::from_utf8_lossy(&self.path))?;
        writeln!(f, "Package Flags: {}", self.pak_flags)?;
        writeln!(f, "Name Count: {}", self.name_count)?;
        writeln!(f, "Export Count: {}", self.export_count)?;
        writeln!(f, "Import Count: {}", self.import_count)?;
        writeln!(f, "Engine Version: {}", self.engine_ver)?;
        writeln!(f, "Cooker Version: {}", self.cooker_ver)?;
        writeln!(f, "Compression Flags: {}", self.compression)?;
        writeln!(f, "GUID: {:?}", self.guid)?;
        writeln!(f, "Generations:")?;
        for (i, gens) in self.gens.iter().enumerate()
        {
            writeln!(
                f, 
                "\tGen {}:\n\t\tExports={}\n\t\tNames={}\n\t\tNetObjs={}", 
                i, gens.export_count, gens.name_count, gens.net_obj_count
            )?;
        }

        Ok(())
    }
}

pub fn upk_read_header<R: Read + Seek>(mut reader: R) -> Result<UpkHeader, Error>
{
    let sig = reader.read_u32::<LittleEndian>()?;
    if sig != 0x9E2A83C1
    {
        return Err(Error::new(ErrorKind::InvalidData, format!("Invalid file, sig=0x{:X}", sig)));
    }

    let pv = reader.read_i16::<LittleEndian>()?;
    let lv = reader.read_i16::<LittleEndian>()?;
    let hs = reader.read_i32::<LittleEndian>()?;
    
    let fl = reader.read_i32::<LittleEndian>()?;
    let mut rfl = fl;
    if fl < 0
    {
        rfl = fl * -2; // needed if utf16
    }
    let mut pn = vec![0u8; rfl as usize];
    reader.read_exact(&mut pn)?;
    
    let pf = reader.read_i32::<LittleEndian>()?;
    
    let nc = reader.read_i32::<LittleEndian>()?;
    let no = reader.read_i32::<LittleEndian>()?;
    let ec = reader.read_i32::<LittleEndian>()?;
    let eo = reader.read_i32::<LittleEndian>()?;
    let ic = reader.read_i32::<LittleEndian>()?;
    let io = reader.read_i32::<LittleEndian>()?;
    let depo = reader.read_i32::<LittleEndian>()?;

    if ic <= 0 || nc <= 0 || ec <= 0
    {
        return Err(Error::new(ErrorKind::InvalidData, "Corrupted pak"));
    }

    let unks =
        [
            reader.read_i32::<LittleEndian>()?,
            reader.read_i32::<LittleEndian>()?,
            reader.read_i32::<LittleEndian>()?,
            reader.read_i32::<LittleEndian>()?,
        ];

    let gid =
        [
            reader.read_i32::<LittleEndian>()?,
            reader.read_i32::<LittleEndian>()?,
            reader.read_i32::<LittleEndian>()?,
            reader.read_i32::<LittleEndian>()?,
        ];

    let gc = reader.read_i32::<LittleEndian>()?;
    let mut gns = Vec::with_capacity(gc as usize);

    for _ in 0..gc
    {
        gns.push(
            GenerationInfo
            {
                export_count: reader.read_i32::<LittleEndian>()?,
                name_count: reader.read_i32::<LittleEndian>()?,
                net_obj_count: reader.read_i32::<LittleEndian>()?
            }
        );
    }

    let ev = reader.read_i32::<LittleEndian>()?;
    let cv = reader.read_i32::<LittleEndian>()?;
    let cf = reader.read_i32::<LittleEndian>()?;

    Ok(
        UpkHeader
        {
            sign: sig,
            p_ver: pv,
            l_ver: lv,
            header_size: hs,
            path_len: fl,
            path: pn,
            pak_flags: pf,
            name_count: nc,
            name_offset: no,
            export_count: ec,
            export_offset: eo,
            import_count: ic,
            import_offset: io,
            depends_offset: depo,
            unk: unks,
            guid: gid,
            gen_count: gc,
            gens: gns,
            engine_ver: ev,
            cooker_ver: cv,
            compression: cf
        }
    )
}
