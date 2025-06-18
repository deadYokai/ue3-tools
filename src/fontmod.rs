use std::{error::Error, fs::File, io::{BufReader, Cursor, Seek, SeekFrom}};

use byteorder::{LittleEndian, ReadBytesExt};

pub fn extract(file: &mut File)
{

    let _ = file.seek(SeekFrom::Start(20));
    let tbl_len = file.read_u32::<LittleEndian>();
    let _ = file.seek(SeekFrom::Current(4));
    
}
