use std::io::{Read, Write, Result};

#[derive(Debug, Clone)]
pub struct PatchData {
    pub data_name: String,
    
    pub data: Vec<u8>,
}

impl PatchData {
    pub fn new(data_name: String, data: Vec<u8>) -> Self {
        Self { data_name, data }
    }

    pub fn serialize<W: Write>(&self, writer: &mut W) -> Result<()> {
        let name_bytes = self.data_name.as_bytes();
        writer.write_all(&(name_bytes.len() as u32).to_le_bytes())?;
        writer.write_all(name_bytes)?;
        
        writer.write_all(&(self.data.len() as u32).to_le_bytes())?;
        writer.write_all(&self.data)?;
        
        Ok(())
    }

    pub fn deserialize<R: Read>(reader: &mut R) -> Result<Self> {
        let mut len_buf = [0u8; 4];
        reader.read_exact(&mut len_buf)?;
        let name_len = u32::from_le_bytes(len_buf) as usize;
        
        let mut name_buf = vec![0u8; name_len];
        reader.read_exact(&mut name_buf)?;
        let data_name = String::from_utf8(name_buf)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        
        reader.read_exact(&mut len_buf)?;
        let data_len = u32::from_le_bytes(len_buf) as usize;
        
        let mut data = vec![0u8; data_len];
        reader.read_exact(&mut data)?;
        
        Ok(Self { data_name, data })
    }
}

#[derive(Debug, Clone)]
pub struct ScriptPatchData {
    pub struct_name: String,
    
    pub patch_data: PatchData,
}

impl ScriptPatchData {
    pub fn new(struct_name: String, function_path: String, bytecode: Vec<u8>) -> Self {
        Self {
            struct_name,
            patch_data: PatchData::new(function_path, bytecode),
        }
    }

    pub fn serialize<W: Write>(&self, writer: &mut W) -> Result<()> {
        let name_bytes = self.struct_name.as_bytes();
        writer.write_all(&(name_bytes.len() as u32).to_le_bytes())?;
        writer.write_all(name_bytes)?;
        
        self.patch_data.serialize(writer)?;
        
        Ok(())
    }

    pub fn deserialize<R: Read>(reader: &mut R) -> Result<Self> {
        let mut len_buf = [0u8; 4];
        reader.read_exact(&mut len_buf)?;
        let name_len = u32::from_le_bytes(len_buf) as usize;
        
        let mut name_buf = vec![0u8; name_len];
        reader.read_exact(&mut name_buf)?;
        let struct_name = String::from_utf8(name_buf)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        
        let patch_data = PatchData::deserialize(reader)?;
        
        Ok(Self {
            struct_name,
            patch_data,
        })
    }

    pub fn get_function_name(&self) -> &str {
        self.patch_data.data_name
            .rsplit('.')
            .next()
            .unwrap_or(&self.patch_data.data_name)
    }
}

#[derive(Debug, Clone)]
pub struct EnumPatchData {
    pub enum_name: String,
    
    pub enum_path_name: String,
    
    pub enum_values: Vec<String>,
}

impl EnumPatchData {
    pub fn new(enum_name: String, enum_path_name: String, enum_values: Vec<String>) -> Self {
        Self {
            enum_name,
            enum_path_name,
            enum_values,
        }
    }

    pub fn serialize<W: Write>(&self, writer: &mut W) -> Result<()> {
        let name_bytes = self.enum_name.as_bytes();
        writer.write_all(&(name_bytes.len() as u32).to_le_bytes())?;
        writer.write_all(name_bytes)?;
        
        let path_bytes = self.enum_path_name.as_bytes();
        writer.write_all(&(path_bytes.len() as u32).to_le_bytes())?;
        writer.write_all(path_bytes)?;
        
        writer.write_all(&(self.enum_values.len() as u32).to_le_bytes())?;
        
        for value in &self.enum_values {
            let value_bytes = value.as_bytes();
            writer.write_all(&(value_bytes.len() as u32).to_le_bytes())?;
            writer.write_all(value_bytes)?;
        }
        
        Ok(())
    }

    pub fn deserialize<R: Read>(reader: &mut R) -> Result<Self> {
        let mut len_buf = [0u8; 4];
        
        reader.read_exact(&mut len_buf)?;
        let name_len = u32::from_le_bytes(len_buf) as usize;
        let mut name_buf = vec![0u8; name_len];
        reader.read_exact(&mut name_buf)?;
        let enum_name = String::from_utf8(name_buf)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        
        reader.read_exact(&mut len_buf)?;
        let path_len = u32::from_le_bytes(len_buf) as usize;
        let mut path_buf = vec![0u8; path_len];
        reader.read_exact(&mut path_buf)?;
        let enum_path_name = String::from_utf8(path_buf)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        
        reader.read_exact(&mut len_buf)?;
        let values_count = u32::from_le_bytes(len_buf) as usize;
        
        let mut enum_values = Vec::with_capacity(values_count);
        for _ in 0..values_count {
            reader.read_exact(&mut len_buf)?;
            let value_len = u32::from_le_bytes(len_buf) as usize;
            let mut value_buf = vec![0u8; value_len];
            reader.read_exact(&mut value_buf)?;
            let value = String::from_utf8(value_buf)
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
            enum_values.push(value);
        }
        
        Ok(Self {
            enum_name,
            enum_path_name,
            enum_values,
        })
    }
}

#[derive(Debug, Clone)]
pub struct LinkerPatchData {
    pub package_name: String,
    
    pub names: Vec<String>,
    
    pub script_patches: Vec<ScriptPatchData>,
    
    pub modified_class_default_objects: Vec<PatchData>,
    
    pub modified_enums: Vec<EnumPatchData>,
    
    pub new_objects: Vec<PatchData>,
}

impl LinkerPatchData {
    pub fn new(package_name: String) -> Self {
        Self {
            package_name,
            names: Vec::new(),
            script_patches: Vec::new(),
            modified_class_default_objects: Vec::new(),
            modified_enums: Vec::new(),
            new_objects: Vec::new(),
        }
    }

    pub fn add_script_patch(&mut self, patch: ScriptPatchData) {
        self.script_patches.push(patch);
    }

    pub fn add_cdo_patch(&mut self, patch: PatchData) {
        self.modified_class_default_objects.push(patch);
    }

    pub fn add_enum_patch(&mut self, patch: EnumPatchData) {
        self.modified_enums.push(patch);
    }

    pub fn serialize<W: Write>(&self, writer: &mut W) -> Result<()> {
        let name_bytes = self.package_name.as_bytes();
        writer.write_all(&(name_bytes.len() as u32).to_le_bytes())?;
        writer.write_all(name_bytes)?;
        
        writer.write_all(&(self.names.len() as u32).to_le_bytes())?;
        for name in &self.names {
            let name_bytes = name.as_bytes();
            writer.write_all(&(name_bytes.len() as u32).to_le_bytes())?;
            writer.write_all(name_bytes)?;
        }
        
        writer.write_all(&(self.new_objects.len() as u32).to_le_bytes())?;
        for obj in &self.new_objects {
            obj.serialize(writer)?;
        }
        
        writer.write_all(&(self.modified_class_default_objects.len() as u32).to_le_bytes())?;
        for cdo in &self.modified_class_default_objects {
            cdo.serialize(writer)?;
        }
        
        writer.write_all(&(self.modified_enums.len() as u32).to_le_bytes())?;
        for enum_patch in &self.modified_enums {
            enum_patch.serialize(writer)?;
        }
        
        writer.write_all(&(self.script_patches.len() as u32).to_le_bytes())?;
        for patch in &self.script_patches {
            patch.serialize(writer)?;
        }
        
        Ok(())
    }

    pub fn deserialize<R: Read>(reader: &mut R) -> Result<Self> {
        let mut len_buf = [0u8; 4];
        
        reader.read_exact(&mut len_buf)?;
        let name_len = u32::from_le_bytes(len_buf) as usize;
        let mut name_buf = vec![0u8; name_len];
        reader.read_exact(&mut name_buf)?;
        let package_name = String::from_utf8(name_buf)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        
        reader.read_exact(&mut len_buf)?;
        let names_count = u32::from_le_bytes(len_buf) as usize;
        let mut names = Vec::with_capacity(names_count);
        for _ in 0..names_count {
            reader.read_exact(&mut len_buf)?;
            let len = u32::from_le_bytes(len_buf) as usize;
            let mut buf = vec![0u8; len];
            reader.read_exact(&mut buf)?;
            names.push(String::from_utf8(buf)
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?);
        }
        
        reader.read_exact(&mut len_buf)?;
        let obj_count = u32::from_le_bytes(len_buf) as usize;
        let mut new_objects = Vec::with_capacity(obj_count);
        for _ in 0..obj_count {
            new_objects.push(PatchData::deserialize(reader)?);
        }
        
        reader.read_exact(&mut len_buf)?;
        let cdo_count = u32::from_le_bytes(len_buf) as usize;
        let mut modified_class_default_objects = Vec::with_capacity(cdo_count);
        for _ in 0..cdo_count {
            modified_class_default_objects.push(PatchData::deserialize(reader)?);
        }
        
        reader.read_exact(&mut len_buf)?;
        let enum_count = u32::from_le_bytes(len_buf) as usize;
        let mut modified_enums = Vec::with_capacity(enum_count);
        for _ in 0..enum_count {
            modified_enums.push(EnumPatchData::deserialize(reader)?);
        }
        
        reader.read_exact(&mut len_buf)?;
        let patch_count = u32::from_le_bytes(len_buf) as usize;
        let mut script_patches = Vec::with_capacity(patch_count);
        for _ in 0..patch_count {
            script_patches.push(ScriptPatchData::deserialize(reader)?);
        }
        
        Ok(Self {
            package_name,
            names,
            script_patches,
            modified_class_default_objects,
            modified_enums,
            new_objects,
        })
    }
}

