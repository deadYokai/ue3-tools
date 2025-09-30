use std::{env, fs::{self, File, OpenOptions}, io::{BufReader, BufWriter, Cursor, Read, Result, Seek, SeekFrom, Write}, path::Path, process::exit};

use ron::ser::{to_string_pretty, PrettyConfig};
use upkreader::parse_upk;

mod upkreader;
mod upkdecompress;
mod fontmod;

fn fontext(filepath: &str)
{
    let path = Path::new(filepath);
    let mut file = match File::open(path)
    {
        Ok(f) => f,
        Err(e) =>
        {
            eprintln!("Failed to open {}", e);
            return;
        }
    };

    fontmod::extract(&mut file);

}

fn upk_header_cursor(path: &str) -> Result<(Cursor<Vec<u8>>, upkreader::UpkHeader)>
{

    let path = Path::new(path);

    let file = File::open(path)?;

    let mut reader = BufReader::new(file);

    let header = upkreader::upk_read_header(&mut reader)?;
    println!("{}", header);
    reader.seek(SeekFrom::Start(size_of::<upkreader::UpkHeader>() as u64))?;

    if header.compression != 0 
    {
        println!("Decompression: {:?}", upkdecompress::parse_chunk_header(&mut reader, &header));
        exit(-1);
    }

    reader.seek(SeekFrom::Start(0))?;
    let mut buf = Vec::new();
    reader.read_to_end(&mut buf)?;
    Ok((Cursor::new(buf), header))
}

fn getlist(path: &str) -> Result<()>
{
    let (cursor, header): (Cursor<Vec<u8>>, upkreader::UpkHeader) = upk_header_cursor(path)?;
    let mut cur: Cursor<&Vec<u8>> = Cursor::new(cursor.get_ref());

    let pak = parse_upk(&mut cur, &header)?;
    let list = upkreader::list_full_obj_paths(&pak);
    for (i, path) in list.iter().enumerate()
    {
        println!("#{} {}", i, path);
    }

    Ok(())
}

fn el(path: &str, names_path: &str) -> Result<()>
{

    let nm_data = fs::read_to_string(names_path)?;
    let name_table: Vec<String> = nm_data.lines().map(|line| line.trim().to_string()).collect();
    let el_data = fs::read(path)?;
    let mut cursor = Cursor::new(&el_data);

    loop
    {
        let _tag = upkreader::read_proptag(&mut cursor, &name_table)?;

        match _tag
        {
            None => break,
            Some(tag) =>
            {
                let v = upkreader::parse_prop_val(&mut cursor, &tag, &name_table)?;
                let pn = &name_table[tag.name_idx as usize];

                println!("{} = {}", pn, v);
            }  
        }
    }
    Ok(())
}

fn dump_names(upk_path: &str, mut output_path: &str) -> Result<()>
{

    if output_path.is_empty()
    {
        output_path = "names_table.txt";
    }

    let (cursor, header): (Cursor<Vec<u8>>, upkreader::UpkHeader) = upk_header_cursor(upk_path)?;
    let mut cur: Cursor<&Vec<u8>> = Cursor::new(cursor.get_ref());
    cur.seek(SeekFrom::Start(header.name_offset as u64))?;

    println!("Names: (count = {})", header.name_count);

    let nt_file = File::create(Path::new(output_path))?;
    let mut writer = BufWriter::new(nt_file);

    for i in 0..header.name_count
    {
        // if i == 0
        // {
        //     println!("Name[{}]: NULL", i);
        //     writeln!(writer, "NULL")?;
        //     continue;
        // }
        let s = upkreader::read_name(&mut cur)?;
        println!("Name[{}]: {}", i, s.name);
        writeln!(writer, "{}", s.name)?;
    }

    Ok(())
}

fn extract_file(upk_path: &str, path: &str, mut output_dir: &str, all: bool) -> Result<()> {
    
    if output_dir.is_empty()
    {
        output_dir = "output";
    }

    let output_dir_path = Path::new(output_dir);
    
    let filename = Path::new(upk_path).file_stem().unwrap();

    
    let pbuf = output_dir_path.join(filename);
    let dir_path: &Path = pbuf.as_path();

    let (mut cursor, header): (Cursor<Vec<u8>>, upkreader::UpkHeader) = upk_header_cursor(upk_path)?;
    let mut cur: Cursor<&Vec<u8>> = Cursor::new(cursor.get_ref());
    let up = upkreader::parse_upk(&mut cur, &header)?;

    if !dir_path.exists() {
        std::fs::create_dir_all(dir_path)?;
    }
    
    let mut data_file = File::create(pbuf.with_extension("ron"))?;

    let pretty = PrettyConfig::new().struct_names(true);

    let s = to_string_pretty(&header, pretty.clone()).expect("Fail");
    writeln!(data_file, "{s}")?;

    let s = to_string_pretty(&up, pretty).expect("Fail");
    writeln!(data_file, "{s}")?;

    upkreader::extract_by_name(&mut cursor, &up, path, dir_path, all)?;

    Ok(())
}

fn pack_upk(_ron_path: &str) -> Result<()> {
    
    Ok(())
}

fn main() -> Result<()> 
{

    let args: Vec<String> = env::args().collect();

    if args.len() <= 1
    {
        println!("No args!");
        exit(0);
    }

    let key = &args[1];
    let mut a2 = "";
    let mut a3 = "";
    let mut a4 = "";

    if args.len() > 2
    {
        a2 = &args[2];
    }

    if args.len() > 3
    {
        a3 = &args[3];
    }

    if args.len() > 4
    {
        a4 = &args[4];
    }

    match key.as_str()
    {
        "fontext"       => fontext(a2),
        "upkHeader"     => { upk_header_cursor(a2)?; }
        "element"       => el(a2, a3)?,
        "list"          => getlist(a2)?,
        "names"         => dump_names(a2, a3)?,
        "extract"       => extract_file(a2, a3, a4, false)?,
        "extractall"    => extract_file(a2, "", a3, true)?,
        "pack"          => pack_upk(a2)?,
        _               => println!("unknown")
    }
    Ok(())
}
