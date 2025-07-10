use std::{collections::HashMap, fmt, io::{Cursor, Error, ErrorKind, Read, Result, Seek}};
use byteorder::{LittleEndian, ReadBytesExt};

pub struct Names
{
    pub n_len: i32,
    pub is_utf16: bool,
    pub name_bytes: Vec<u8>,
    pub name: String,
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
    pub name_count: i32,
    pub name_offset: i32,
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

pub enum UE3Prop
{
    Array(Vec<UE3Prop>),
    Bool(bool),
    Byte(u8),
    Int(i32),
    Float(f32),
    Str(String),
    Struct(HashMap<String, UE3Prop>),
    Object(u32),
    Name(u32),
    Unknown(String)
}

pub struct UE3Proptag
{
    pub name_idx: u32,
    pub type_name: String,
    pub size: u32,
    pub array_index: u32
}

pub struct UPKPak
{
    pub name_table: Vec<String>,
    pub export_table: Vec<Export>,
    pub import_table: Vec<Import>,
}

pub fn parse_upk(cursor: &mut Cursor<&Vec<u8>>, header: &UpkHeader) -> Result<UPKPak>
{
    let name_count = header.name_count;
    let name_offset = header.name_offset;
    let export_count = header.export_count;
    let export_offset = header.export_offset;
    let import_count = header.import_count;
    let import_offset = header.import_offset;

    let mut name_table = Vec::new();
    cursor.set_position(name_offset as u64);
    for _ in 0..name_count
    {
        let name = read_name(cursor)?;
        name_table.push(name.name);
    }

    let mut export_table = Vec::new();
    cursor.set_position(export_offset as u64);
    for _ in 0..export_count
    {
        let obj_type_ref = cursor.read_i32::<LittleEndian>()?;
        let parent_class_ref = cursor.read_i32::<LittleEndian>()?;
        let owner_ref = cursor.read_i32::<LittleEndian>()?;
        let name_tbl_idx = cursor.read_i32::<LittleEndian>()?;
        let name_count = cursor.read_i32::<LittleEndian>()?;
        let field6 = cursor.read_i32::<LittleEndian>()?;
        let obj_flags_h = cursor.read_i32::<LittleEndian>()?;
        let obj_flags_l = cursor.read_i32::<LittleEndian>()?;
        let obj_filesize = cursor.read_i32::<LittleEndian>()?;
        let data_offset = cursor.read_i32::<LittleEndian>()?;
        let field11 = cursor.read_i32::<LittleEndian>()?;
        let num_additional_fields = cursor.read_i32::<LittleEndian>()?;

        let mut unk_fields = Vec::new();
        for _ in 0..num_additional_fields {
            unk_fields.push(cursor.read_i32::<LittleEndian>()?);
        }

        let field13 = cursor.read_i32::<LittleEndian>()?;
        let field14 = cursor.read_i32::<LittleEndian>()?;
        let field15 = cursor.read_i32::<LittleEndian>()?;
        let field16 = cursor.read_i32::<LittleEndian>()?;
        let field17 = cursor.read_i32::<LittleEndian>()?;

        export_table.push(Export {
            obj_type_ref,
            parent_class_ref,
            owner_ref,
            name_tbl_idx,
            name_count,
            field6,
            obj_flags_h,
            obj_flags_l,
            obj_filesize,
            data_offset,
            field11,
            num_additional_fields,
            field13,
            field14,
            field15,
            field16,
            field17,
            unk_fields,
        });

    }

    // package_idx: i32,
    // unk1: i32,
    // obj_type_idx: i32,
    // unk2: i32,
    // owner_ref: i32,
    // name_tbl_idx: i32,
    // unk3: i32

    let mut import_table = Vec::new();

    cursor.set_position(import_offset as u64);
    for _ in 0..import_count
    {
        let package_idx = cursor.read_i32::<LittleEndian>()?;
        let unk1 = cursor.read_i32::<LittleEndian>()?;
        let obj_type_idx = cursor.read_i32::<LittleEndian>()?;
        let unk2 = cursor.read_i32::<LittleEndian>()?;
        let owner_ref = cursor.read_i32::<LittleEndian>()?;
        let name_tbl_idx = cursor.read_i32::<LittleEndian>()?;
        let unk3 = cursor.read_i32::<LittleEndian>()?;

        import_table.push(Import { package_idx, unk1, obj_type_idx, unk2, owner_ref, name_tbl_idx, unk3 });
    }

    Ok(UPKPak{name_table, export_table, import_table})
}

pub fn resolve_type_name(obj_type_ref: i32, pkg: &UPKPak) -> String {
    if obj_type_ref < 0 {
        let import_index = (-obj_type_ref - 1) as usize;
        if import_index < pkg.import_table.len() {
            let import = &pkg.import_table[import_index];
            if (import.name_tbl_idx as usize) < pkg.name_table.len() {
                return pkg.name_table[import.name_tbl_idx as usize].clone();
            }
        }
    } else if obj_type_ref > 0 {
        let export_index = (obj_type_ref - 1) as usize;
        if export_index < pkg.export_table.len() {
            let export = &pkg.export_table[export_index];
            if (export.name_tbl_idx as usize) < pkg.name_table.len() {
                return pkg.name_table[export.name_tbl_idx as usize].clone();
            }
        }
    }

    "unk".to_string()
}


pub fn list_full_obj_paths(pkg: UPKPak) -> Vec<String>
{
    let mut paths = Vec::new();

    for (idx, _) in pkg.export_table.iter().enumerate()
    {
        let mut path_parts = Vec::new();
        let mut current = Some(idx as i32 + 1);

        while let Some(i) = current
        {
            if i <= 0
            {
                break;
            }

            let exp = &pkg.export_table[i as usize - 1];

            let mut name = pkg.name_table
                .get(exp.name_tbl_idx as usize)
                .cloned().unwrap_or_else(|| "<invalid>".to_string());

            
            if exp.name_count > 0
            {
                name = format!("{}_{}", name, exp.name_count - 1);
            }

            let extension = resolve_type_name(exp.obj_type_ref, &pkg);
            name = format!("{}.{}", name, extension);

            path_parts.push(name);

            current = Some(exp.owner_ref);
        }

        path_parts.reverse();
        paths.push(path_parts.join("/"));
    }

    paths
}

pub fn read_name(cursor: &mut Cursor<&Vec<u8>>) -> Result<Names>
{
    let len = cursor.read_i32::<LittleEndian>()?;
    
    if len == 0
    {
        return Ok(Names{n_len: 0, is_utf16: false, name: "".to_string(), name_bytes: Vec::new(), n_fh: 0, n_fl: 0})
    }

    if len > 0
    {
        let mut buf = vec![0u8; len as usize];
        cursor.read_exact(&mut buf)?;

        let n_fh = cursor.read_i32::<LittleEndian>()?;
        let n_fl = cursor.read_i32::<LittleEndian>()?;

        if buf.last() == Some(&0)
        {
            buf.pop();
        }

        // let name = String::from_utf8(buf.clone())
        //     .map_err(|_| Error::new(ErrorKind::InvalidData, format!("Invalid UTF8 {:x?}", buf)))?;

        let name = buf.iter().map(|&b| b as char).collect::<String>(); // not utf8 but ISO-8859-1

        Ok(Names
        {
            n_len: len, is_utf16: false, name, name_bytes: buf, n_fh, n_fl
        })

    } else {
        let wchar_count = -len;
        let mut buf = vec![0u8; (wchar_count * 2) as usize];
        cursor.read_exact(&mut buf)?;

        let n_fh = cursor.read_i32::<LittleEndian>()?;
        let n_fl = cursor.read_i32::<LittleEndian>()?;

        let utf16: Vec<u16> = buf
            .chunks_exact(2)
            .map(|chunk| u16::from_le_bytes([chunk[0], chunk[1]]))
            .collect();

        let utf16_trimmed = match utf16.last()
        {
            Some(&0) => &utf16[..utf16.len() - 1],
            _ => &utf16[..]
        };

        let name = String::from_utf16(utf16_trimmed)
            .map_err(|_| Error::new(ErrorKind::InvalidData, "Invalid UTF16"))?;
        Ok(Names
        {
            n_len: wchar_count, is_utf16: true, name, name_bytes: buf, n_fh, n_fl
        })
    }
}


pub fn read_string(cursor: &mut Cursor<&Vec<u8>>) -> Result<String>
{
    let len = cursor.read_i32::<LittleEndian>()?;
    if len == 0
    {
        return Ok("".to_string());
    }

    if len > 0
    {
        let mut buf = vec![0u8; len as usize];
        cursor.read_exact(&mut buf)?;

        if buf.last() == Some(&0)
        {
            buf.pop();
        }

        //String::from_utf8(buf.clone())
        //     .map_err(|_| Error::new(ErrorKind::InvalidData, format!("Invalid UTF8 {:x?}", buf)))

        Ok(buf.iter().map(|&b| b as char).collect::<String>()) // not utf8 but ISO-8859-1
    } else {
        let wchar_count = -len;
        let mut buf = vec![0u8; (wchar_count * 2) as usize];
        cursor.read_exact(&mut buf)?;

        let utf16: Vec<u16> = buf
            .chunks_exact(2)
            .map(|chunk| u16::from_le_bytes([chunk[0], chunk[1]]))
            .collect();

        let utf16_trimmed = match utf16.last()
        {
            Some(&0) => &utf16[..utf16.len() - 1],
            _ => &utf16[..]
        };

        String::from_utf16(utf16_trimmed)
            .map_err(|_| Error::new(ErrorKind::InvalidData, "Invalid UTF16"))
    }
}


pub fn read_proptag(cursor: &mut Cursor<&Vec<u8>>, name_table: &[String]) -> Result<Option<UE3Proptag>>
{
    let name_idx = cursor.read_u32::<LittleEndian>()?;
    let name = name_table.get(name_idx as usize)
        .ok_or(Error::new(ErrorKind::InvalidData, format!("Invalid name index {}", name_idx)))?;

    if name == "None"
    {
        return Ok(None);
    }

    let type_name_index = cursor.read_u32::<LittleEndian>()?;
    let type_name = name_table.get(type_name_index as usize)
        .ok_or(Error::new(ErrorKind::InvalidData, format!("Invalid type name index {}", type_name_index)))
        .unwrap().clone();

    let size = cursor.read_u32::<LittleEndian>()?;
    let array_index = cursor.read_u32::<LittleEndian>()?;
    Ok(Some(UE3Proptag { name_idx, type_name, size, array_index }))
}

fn get_arr_el_type(prop: &str) -> UE3Proptag
{
    let type_name = match prop {
        "Characters" | "Kerning" |
        "TextureCoordinates" | "Vertices" |
        "Sockets" | "Points" | "Normals" |
        "Tangents" | "UVs" | "ExtraUVs" |
        "InstanceData" => "StructProperty",

        "Names" => "NameProperty",
        "Indices" => "IntProperty",
        "Materials" | "Sounds" | "ChildComponents" => "ObjectProperty",

        name if name.ends_with("Names") => "NameProperty",
        name if name.ends_with("Objects") => "ObjectProperty",
        name if name.ends_with("Indices") => "IntProperty",
        name if name.ends_with("Floats") => "FloatProperty",
        name if name.ends_with("Bools") => "BoolProperty",
        _ => "IntProperty"
    };

    UE3Proptag {
        name_idx: 0,
        type_name: type_name.to_string(),
        size: 0,
        array_index: 0
    }
}

pub fn parse_prop_val(
    cursor: &mut Cursor<&Vec<u8>>,
    tag: &UE3Proptag,
    name_table: &[String]
    ) -> Result<UE3Prop>
{
    println!("prop {}", tag.type_name.as_str());
    
    match get_arr_el_type(tag.type_name.as_str()).type_name.as_str()
    {
        "IntProperty"    => Ok(UE3Prop::Int(cursor.read_i32::<LittleEndian>()?)),
        "FloatProperty"  => Ok(UE3Prop::Float(cursor.read_f32::<LittleEndian>()?)),
        "BoolProperty"   => Ok(UE3Prop::Bool(cursor.read_u8()? != 0)),
        "ByteProperty"   => Ok(UE3Prop::Byte(cursor.read_u8()?)),
        "StrProperty"    => Ok(UE3Prop::Str(read_string(cursor)?)),
        "NameProperty"   => Ok(UE3Prop::Name(cursor.read_u32::<LittleEndian>()?)),
        "ObjectProperty" => Ok(UE3Prop::Object(cursor.read_u32::<LittleEndian>()?)),

        "ArrayProperty"  =>
        {
            let inner_count = cursor.read_u32::<LittleEndian>()?;
            let et = get_arr_el_type(name_table[tag.name_idx as usize].as_str());
            println!(
                "ArrayProperty: name = {}, resolved type = {}, count = {}",
                name_table[tag.name_idx as usize], et.type_name.as_str(), inner_count
            );
            let arr = parse_arr_prop(cursor, et.type_name.as_str(), inner_count, name_table)?;
            Ok(UE3Prop::Array(arr))
        }
        "StructProperty" => {
            println!("Struct?");
            let _size = cursor.read_u32::<LittleEndian>()?;

            let mut fields = HashMap::new();

            loop
            {
                let tag_o = read_proptag(cursor, name_table)?;
                match tag_o
                {
                    None => break,
                    Some(tag) =>
                    {
                        let val = parse_prop_val(cursor, &tag, name_table)?;
                        let name = name_table.get(tag.name_idx as usize)
                            .ok_or(Error::new(ErrorKind::InvalidData, format!("Invalid name index {}", tag.name_idx)))?;
                        fields.insert(name.clone(), val);
                    }
                }
            }
            Ok(UE3Prop::Struct(fields))
        }
        other => {
            eprintln!("Warning: Unknown property type '{}'", other);
            Ok(UE3Prop::Unknown(other.to_string()))
        }
    }
}

fn parse_arr_prop(
    cursor: &mut Cursor<&Vec<u8>>,
    el_type: &str,
    count: u32,
    name_table: &[String]
) -> Result<Vec<UE3Prop>>
{
    let mut elements = Vec::with_capacity(count as usize);
    for _ in 0..count
    {
        let dummy = UE3Proptag
        {
            name_idx: 0,
            type_name: el_type.to_string(),
            size: 0,
            array_index: 0
        };

        let element = parse_prop_val(cursor, &dummy, name_table)?;
        elements.push(element);
    }

    Ok(elements)
}


impl fmt::Display for UE3Prop
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self
        {
            UE3Prop::Array(arr) => {
                write!(f, "[")?;
                for (i, el) in arr.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}", el)?;
                }
                write!(f, "]")
            }
            UE3Prop::Bool(b) => write!(f, "{}", b),
            UE3Prop::Byte(b) => write!(f, "0x{:02X}", b),
            UE3Prop::Int(i) => write!(f, "{}", i),
            UE3Prop::Float(fl) => write!(f, "{}", fl),
            UE3Prop::Str(s) => write!(f, "\"{}\"", s),
            UE3Prop::Struct(map) => {
                write!(f, "{{")?;
                let mut first = true;
                for (k, v) in map {
                    if !first {
                        write!(f, ", ")?;
                    }
                    first = false;
                    write!(f, "{}: {}", k, v)?;
                }
                write!(f, "}}")
            }
            UE3Prop::Object(o) => write!(f, "Object({})", o),
            UE3Prop::Name(n) => write!(f, "Name({})", n),
            UE3Prop::Unknown(s) => write!(f, "Unknown({})", s),
        }
    }
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
        writeln!(f, "GUID: {:x?}", self.guid)?;
        if self.gen_count > 0
        {
            writeln!(f, "Generations (Count={}):", self.gen_count)?;
        }
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

pub fn upk_read_header<R: Read + Seek>(mut reader: R) -> Result<UpkHeader>
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

    let header = UpkHeader
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
    };

    Ok(header)
}
