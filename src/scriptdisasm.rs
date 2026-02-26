use std::io::{Cursor, Read, Result, Seek, SeekFrom};
use byteorder::{LittleEndian, ReadBytesExt};
use crate::upkreader::UPKPak;

// ── Token enum ────────────────────────────────────────────────────────────────

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExprToken {
    LocalVariable       = 0x00,
    InstanceVariable    = 0x01,
    DefaultVariable     = 0x02,
    StateVariable       = 0x03,
    Return              = 0x04,
    Switch              = 0x05,
    Jump                = 0x06,
    JumpIfNot           = 0x07,
    Stop                = 0x08,
    Assert              = 0x09,
    Case                = 0x0A,
    Nothing             = 0x0B,
    LabelTable          = 0x0C,
    GotoLabel           = 0x0D,
    EatReturnValue      = 0x0E,
    Let                 = 0x0F,
    DynArrayElement     = 0x10,
    New                 = 0x11,
    ClassContext        = 0x12,
    MetaCast            = 0x13,
    LetBool             = 0x14,
    EndParmValue        = 0x15,
    EndFunctionParms    = 0x16,
    Self_               = 0x17,
    Skip                = 0x18,
    Context             = 0x19,
    ArrayElement        = 0x1A,
    VirtualFunction     = 0x1B,
    FinalFunction       = 0x1C,
    IntConst            = 0x1D,
    FloatConst          = 0x1E,
    StringConst         = 0x1F,
    ObjectConst         = 0x20,
    NameConst           = 0x21,
    RotationConst       = 0x22,
    VectorConst         = 0x23,
    ByteConst           = 0x24,
    IntZero             = 0x25,
    IntOne              = 0x26,
    True                = 0x27,
    False               = 0x28,
    NativeParm          = 0x29,
    NoObject            = 0x2A,
    IntConstByte        = 0x2C,
    BoolVariable        = 0x2D,
    DynamicCast         = 0x2E,
    Iterator            = 0x2F,
    IteratorPop         = 0x30,
    IteratorNext        = 0x31,
    StructCmpEq         = 0x32,
    StructCmpNe         = 0x33,
    UnicodeStringConst  = 0x34,
    StructMember        = 0x35,
    DynArrayLength      = 0x36,
    GlobalFunction      = 0x37,
    PrimitiveCast       = 0x38,
    DynArrayInsert      = 0x39,
    ReturnNothing       = 0x3A,
    EqualEqual_DelDel   = 0x3B,
    NotEqual_DelDel     = 0x3C,
    EqualEqual_DelFunc  = 0x3D,
    NotEqual_DelFunc    = 0x3E,
    EmptyDelegate       = 0x3F,
    DynArrayRemove      = 0x40,
    DebugInfo           = 0x41,
    DelegateFunction    = 0x42,
    DelegateProperty    = 0x43,
    LetDelegate         = 0x44,
    Conditional         = 0x45,
    DynArrayFind        = 0x46,
    DynArrayFindStruct  = 0x47,
    LocalOutVariable    = 0x48,
    DefaultParmValue    = 0x49,
    EmptyParmValue      = 0x4A,
    InstanceDelegate    = 0x4B,
    InterfaceContext    = 0x51,
    InterfaceCast       = 0x52,
    EndOfScript         = 0x53,
    DynArrayAdd         = 0x54,
    DynArrayAddItem     = 0x55,
    DynArrayRemoveItem  = 0x56,
    DynArrayInsertItem  = 0x57,
    DynArrayIterator    = 0x58,
    DynArraySort        = 0x59,
    JumpIfFilterEditorOnly = 0x5A,
    ExtendedNative      = 0x60,
    Unknown             = 0xFF,
}

impl ExprToken {
    pub fn from_byte(b: u8) -> Self {
        match b {
            0x00 => Self::LocalVariable,
            0x01 => Self::InstanceVariable,
            0x02 => Self::DefaultVariable,
            0x03 => Self::StateVariable,
            0x04 => Self::Return,
            0x05 => Self::Switch,
            0x06 => Self::Jump,
            0x07 => Self::JumpIfNot,
            0x08 => Self::Stop,
            0x09 => Self::Assert,
            0x0A => Self::Case,
            0x0B => Self::Nothing,
            0x0C => Self::LabelTable,
            0x0D => Self::GotoLabel,
            0x0E => Self::EatReturnValue,
            0x0F => Self::Let,
            0x10 => Self::DynArrayElement,
            0x11 => Self::New,
            0x12 => Self::ClassContext,
            0x13 => Self::MetaCast,
            0x14 => Self::LetBool,
            0x15 => Self::EndParmValue,
            0x16 => Self::EndFunctionParms,
            0x17 => Self::Self_,
            0x18 => Self::Skip,
            0x19 => Self::Context,
            0x1A => Self::ArrayElement,
            0x1B => Self::VirtualFunction,
            0x1C => Self::FinalFunction,
            0x1D => Self::IntConst,
            0x1E => Self::FloatConst,
            0x1F => Self::StringConst,
            0x20 => Self::ObjectConst,
            0x21 => Self::NameConst,
            0x22 => Self::RotationConst,
            0x23 => Self::VectorConst,
            0x24 => Self::ByteConst,
            0x25 => Self::IntZero,
            0x26 => Self::IntOne,
            0x27 => Self::True,
            0x28 => Self::False,
            0x29 => Self::NativeParm,
            0x2A => Self::NoObject,
            0x2C => Self::IntConstByte,
            0x2D => Self::BoolVariable,
            0x2E => Self::DynamicCast,
            0x2F => Self::Iterator,
            0x30 => Self::IteratorPop,
            0x31 => Self::IteratorNext,
            0x32 => Self::StructCmpEq,
            0x33 => Self::StructCmpNe,
            0x34 => Self::UnicodeStringConst,
            0x35 => Self::StructMember,
            0x36 => Self::DynArrayLength,
            0x37 => Self::GlobalFunction,
            0x38 => Self::PrimitiveCast,
            0x39 => Self::DynArrayInsert,
            0x3A => Self::ReturnNothing,
            0x3B => Self::EqualEqual_DelDel,
            0x3C => Self::NotEqual_DelDel,
            0x3D => Self::EqualEqual_DelFunc,
            0x3E => Self::NotEqual_DelFunc,
            0x3F => Self::EmptyDelegate,
            0x40 => Self::DynArrayRemove,
            0x41 => Self::DebugInfo,
            0x42 => Self::DelegateFunction,
            0x43 => Self::DelegateProperty,
            0x44 => Self::LetDelegate,
            0x45 => Self::Conditional,
            0x46 => Self::DynArrayFind,
            0x47 => Self::DynArrayFindStruct,
            0x48 => Self::LocalOutVariable,
            0x49 => Self::DefaultParmValue,
            0x4A => Self::EmptyParmValue,
            0x4B => Self::InstanceDelegate,
            0x51 => Self::InterfaceContext,
            0x52 => Self::InterfaceCast,
            0x53 => Self::EndOfScript,
            0x54 => Self::DynArrayAdd,
            0x55 => Self::DynArrayAddItem,
            0x56 => Self::DynArrayRemoveItem,
            0x57 => Self::DynArrayInsertItem,
            0x58 => Self::DynArrayIterator,
            0x59 => Self::DynArraySort,
            0x5A => Self::JumpIfFilterEditorOnly,
            0x60..=0x6F => Self::ExtendedNative,
            _ => Self::Unknown,
        }
    }
}

fn cast_name(b: u8) -> &'static str {
    match b {
        0x36 => "InterfaceToObject",
        0x37 => "InterfaceToString",
        0x38 => "InterfaceToBool",
        0x39 => "RotatorToVector",
        0x3A => "ByteToInt",
        0x3B => "ByteToBool",
        0x3C => "ByteToFloat",
        0x3D => "IntToByte",
        0x3E => "IntToBool",
        0x3F => "IntToFloat",
        0x40 => "BoolToByte",
        0x41 => "BoolToInt",
        0x42 => "BoolToFloat",
        0x43 => "FloatToByte",
        0x44 => "FloatToInt",
        0x45 => "FloatToBool",
        0x46 => "ObjectToInterface",
        0x47 => "ObjectToBool",
        0x48 => "NameToBool",
        0x49 => "StringToByte",
        0x4A => "StringToInt",
        0x4B => "StringToBool",
        0x4C => "StringToFloat",
        0x4D => "StringToVector",
        0x4E => "StringToRotator",
        0x4F => "VectorToBool",
        0x50 => "VectorToRotator",
        0x51 => "RotatorToBool",
        0x52 => "ByteToString",
        0x53 => "IntToString",
        0x54 => "BoolToString",
        0x55 => "FloatToString",
        0x56 => "ObjectToString",
        0x57 => "NameToString",
        0x58 => "VectorToString",
        0x59 => "RotatorToString",
        0x5A => "DelegateToString",
        0x60 => "StringToName",
        _ => "UnknownCast",
    }
}

// ── Context ───────────────────────────────────────────────────────────────────

pub struct DisasmCtx<'a> {
    pub pak: &'a UPKPak,
    pub indent: usize,
}

impl<'a> DisasmCtx<'a> {
    pub fn new(pak: &'a UPKPak) -> Self { Self { pak, indent: 0 } }
    fn indented(&self) -> Self { DisasmCtx { pak: self.pak, indent: self.indent + 1 } }
}

// ── String / name helpers ─────────────────────────────────────────────────────

fn read_cstring(c: &mut Cursor<&[u8]>) -> Result<String> {
    let mut s = Vec::new();
    loop {
        let b = c.read_u8()?;
        if b == 0 { break; }
        s.push(b);
    }
    Ok(String::from_utf8_lossy(&s).into_owned())
}

fn read_ustring(c: &mut Cursor<&[u8]>) -> Result<String> {
    let mut chars = Vec::new();
    loop {
        let ch = c.read_u16::<LittleEndian>()?;
        if ch == 0 { break; }
        chars.push(ch);
    }
    Ok(String::from_utf16_lossy(&chars))
}

fn read_fname(c: &mut Cursor<&[u8]>, pak: &UPKPak) -> Result<String> {
    let idx  = c.read_i32::<LittleEndian>()?;
    let inst = c.read_i32::<LittleEndian>()?;
    // name_table is Vec<String>
    let name = pak.name_table
        .get(idx as usize)
        .cloned()
        .unwrap_or_else(|| format!("Name[{}]", idx));
    if inst > 0 {
        Ok(format!("{}_{}", name, inst))
    } else {
        Ok(name)
    }
}

pub fn resolve_obj_ref(idx: i32, pak: &UPKPak) -> String {
    if idx == 0 { return "None".to_string(); }
    if idx > 0 {
        if let Some(e) = pak.export_table.get((idx - 1) as usize) {
            return pak.name_table
                .get(e.object_name.name_index as usize)
                .cloned()
                .unwrap_or_else(|| format!("Export[{}]", idx));
        }
    } else {
        if let Some(i) = pak.import_table.get((-idx - 1) as usize) {
            return pak.name_table
                .get(i.object_name.name_index as usize)
                .cloned()
                .unwrap_or_else(|| format!("Import[{}]", idx));
        }
    }
    format!("ObjRef[{}]", idx)
}

// ── Expression disassembler ───────────────────────────────────────────────────

pub fn disasm_expr(c: &mut Cursor<&[u8]>, ctx: &DisasmCtx) -> Result<String> {
    let pos = c.position();
    let raw = c.read_u8()?;
    let tok = ExprToken::from_byte(raw);

    let expr = match tok {
        ExprToken::LocalVariable | ExprToken::LocalOutVariable => {
            let obj = c.read_i32::<LittleEndian>()?;
            resolve_obj_ref(obj, ctx.pak)
        }
        ExprToken::InstanceVariable => {
            let obj = c.read_i32::<LittleEndian>()?;
            format!("self.{}", resolve_obj_ref(obj, ctx.pak))
        }
        ExprToken::DefaultVariable => {
            let obj = c.read_i32::<LittleEndian>()?;
            format!("Default.{}", resolve_obj_ref(obj, ctx.pak))
        }
        ExprToken::StateVariable => {
            let obj = c.read_i32::<LittleEndian>()?;
            format!("StateVar({})", resolve_obj_ref(obj, ctx.pak))
        }
        ExprToken::BoolVariable | ExprToken::NativeParm => {
            let obj = c.read_i32::<LittleEndian>()?;
            resolve_obj_ref(obj, ctx.pak)
        }
        ExprToken::DelegateProperty | ExprToken::InstanceDelegate => {
            let name = read_fname(c, ctx.pak)?;
            let obj  = c.read_i32::<LittleEndian>()?;
            format!("delegate<{},{}>", name, resolve_obj_ref(obj, ctx.pak))
        }

        ExprToken::Return => {
            let inner = disasm_expr(c, ctx)?;
            if inner == "Nothing" { "return".to_string() }
            else { format!("return {}", inner) }
        }
        ExprToken::ReturnNothing => "return /*nothing*/".to_string(),
        ExprToken::Stop          => "stop".to_string(),
        ExprToken::Nothing       => "Nothing".to_string(),
        ExprToken::EndOfScript   => "// end of script".to_string(),
        ExprToken::EndFunctionParms => "/*EndParms*/".to_string(),
        ExprToken::EndParmValue  => "/*EndParmValue*/".to_string(),
        ExprToken::EmptyParmValue => "/*EmptyParm*/".to_string(),
        ExprToken::IteratorNext  => "IteratorNext".to_string(),
        ExprToken::IteratorPop   => "IteratorPop".to_string(),

        ExprToken::Jump => {
            let off = c.read_u16::<LittleEndian>()?;
            format!("goto 0x{:04X}", off)
        }
        ExprToken::JumpIfNot => {
            let off  = c.read_u16::<LittleEndian>()?;
            let cond = disasm_expr(c, ctx)?;
            format!("if (!{}) goto 0x{:04X}", cond, off)
        }
        ExprToken::JumpIfFilterEditorOnly => {
            let off = c.read_u16::<LittleEndian>()?;
            format!("if (!Editor) goto 0x{:04X}", off)
        }
        ExprToken::GotoLabel => {
            let e = disasm_expr(c, ctx)?;
            format!("goto {}", e)
        }

        ExprToken::Switch => {
            let prop = disasm_expr(c, ctx)?;
            let _sz  = c.read_u8()?;
            format!("switch ({})", prop)
        }
        ExprToken::Case => {
            let off = c.read_u16::<LittleEndian>()?;
            if off == 0xFFFF {
                "default:".to_string()
            } else {
                let val = disasm_expr(c, ctx)?;
                format!("case {}:", val)
            }
        }

        ExprToken::Assert => {
            let line = c.read_u16::<LittleEndian>()?;
            let _dbg = c.read_u8()?;
            let cond = disasm_expr(c, ctx)?;
            format!("assert({}) /* line {} */", cond, line)
        }

        ExprToken::Let | ExprToken::LetBool | ExprToken::LetDelegate => {
            let lhs = disasm_expr(c, ctx)?;
            let rhs = disasm_expr(c, ctx)?;
            format!("{} = {}", lhs, rhs)
        }
        ExprToken::EatReturnValue => {
            let _prop = c.read_i32::<LittleEndian>()?;
            "/*EatReturn*/".to_string()
        }

        ExprToken::IntConst      => format!("{}", c.read_i32::<LittleEndian>()?),
        ExprToken::FloatConst    => format!("{:.6}f", c.read_f32::<LittleEndian>()?),
        ExprToken::ByteConst | ExprToken::IntConstByte => format!("{}", c.read_u8()?),
        ExprToken::IntZero       => "0".to_string(),
        ExprToken::IntOne        => "1".to_string(),
        ExprToken::True          => "true".to_string(),
        ExprToken::False         => "false".to_string(),
        ExprToken::NoObject | ExprToken::EmptyDelegate => "None".to_string(),
        ExprToken::Self_         => "self".to_string(),

        ExprToken::StringConst => {
            let s = read_cstring(c)?;
            format!("\"{}\"", s.replace('"', "\\\""))
        }
        ExprToken::UnicodeStringConst => {
            let s = read_ustring(c)?;
            format!("\"{}\"", s.replace('"', "\\\""))
        }
        ExprToken::NameConst => {
            let name = read_fname(c, ctx.pak)?;
            format!("'{}'", name)
        }
        ExprToken::ObjectConst => {
            let obj   = c.read_i32::<LittleEndian>()?;
            let class = c.read_i32::<LittleEndian>()?;
            format!("{}'{}' ", resolve_obj_ref(class, ctx.pak), resolve_obj_ref(obj, ctx.pak))
        }
        ExprToken::RotationConst => {
            let pitch = c.read_i32::<LittleEndian>()?;
            let yaw   = c.read_i32::<LittleEndian>()?;
            let roll  = c.read_i32::<LittleEndian>()?;
            format!("rot({},{},{})", pitch, yaw, roll)
        }
        ExprToken::VectorConst => {
            let x = c.read_f32::<LittleEndian>()?;
            let y = c.read_f32::<LittleEndian>()?;
            let z = c.read_f32::<LittleEndian>()?;
            format!("vect({:.4},{:.4},{:.4})", x, y, z)
        }

        ExprToken::LabelTable => {
            let mut out = String::from("/*LabelTable:");
            loop {
                let name = read_fname(c, ctx.pak)?;
                let off  = c.read_u16::<LittleEndian>()?;
                if name == "None" { break; }
                out.push_str(&format!(" {}=0x{:04X},", name, off));
            }
            out.push_str("*/");
            out
        }

        ExprToken::VirtualFunction | ExprToken::GlobalFunction => {
            let fname = read_fname(c, ctx.pak)?;
            let args  = disasm_params(c, ctx)?;
            format!("{}({})", fname, args)
        }
        ExprToken::FinalFunction => {
            let obj  = c.read_i32::<LittleEndian>()?;
            let args = disasm_params(c, ctx)?;
            format!("{}({})", resolve_obj_ref(obj, ctx.pak), args)
        }
        ExprToken::DelegateFunction => {
            let _marker = c.read_u8()?;
            let obj  = c.read_i32::<LittleEndian>()?;
            let name = read_fname(c, ctx.pak)?;
            let args = disasm_params(c, ctx)?;
            format!("{}.{}({})", resolve_obj_ref(obj, ctx.pak), name, args)
        }

        ExprToken::Context | ExprToken::ClassContext => {
            let obj_expr   = disasm_expr(c, ctx)?;
            let _skip_size = c.read_u16::<LittleEndian>()?;
            let _var_size  = c.read_u16::<LittleEndian>()?;
            let _var_type  = c.read_u8()?;
            let inner      = disasm_expr(c, ctx)?;
            format!("{}.{}", obj_expr, inner)
        }
        ExprToken::InterfaceContext => disasm_expr(c, ctx)?,

        ExprToken::StructMember => {
            let field  = c.read_i32::<LittleEndian>()?;
            let _owner = c.read_i32::<LittleEndian>()?;
            let _tok   = c.read_u8()?;
            let _rval  = c.read_u8()?;
            let inner  = disasm_expr(c, ctx)?;
            format!("{}.{}", inner, resolve_obj_ref(field, ctx.pak))
        }

        ExprToken::ArrayElement | ExprToken::DynArrayElement => {
            let idx_e = disasm_expr(c, ctx)?;
            let arr_e = disasm_expr(c, ctx)?;
            format!("{}[{}]", arr_e, idx_e)
        }
        ExprToken::DynArrayLength => {
            let arr = disasm_expr(c, ctx)?;
            format!("{}.Length", arr)
        }
        ExprToken::DynArrayAdd => {
            let arr = disasm_expr(c, ctx)?;
            let n   = disasm_expr(c, ctx)?;
            format!("{}.Add({})", arr, n)
        }
        ExprToken::DynArrayAddItem => {
            let arr  = disasm_expr(c, ctx)?;
            let item = disasm_expr(c, ctx)?;
            format!("{}.AddItem({})", arr, item)
        }
        ExprToken::DynArrayInsert => {
            let arr = disasm_expr(c, ctx)?;
            let idx = disasm_expr(c, ctx)?;
            let cnt = disasm_expr(c, ctx)?;
            format!("{}.Insert({}, {})", arr, idx, cnt)
        }
        ExprToken::DynArrayInsertItem => {
            let arr  = disasm_expr(c, ctx)?;
            let idx  = disasm_expr(c, ctx)?;
            let item = disasm_expr(c, ctx)?;
            format!("{}.InsertItem({}, {})", arr, idx, item)
        }
        ExprToken::DynArrayRemove => {
            let arr = disasm_expr(c, ctx)?;
            let idx = disasm_expr(c, ctx)?;
            let cnt = disasm_expr(c, ctx)?;
            format!("{}.Remove({}, {})", arr, idx, cnt)
        }
        ExprToken::DynArrayRemoveItem => {
            let arr  = disasm_expr(c, ctx)?;
            let item = disasm_expr(c, ctx)?;
            format!("{}.RemoveItem({})", arr, item)
        }
        ExprToken::DynArrayFind => {
            let arr = disasm_expr(c, ctx)?;
            let val = disasm_expr(c, ctx)?;
            format!("{}.Find({})", arr, val)
        }
        ExprToken::DynArrayFindStruct => {
            let arr  = disasm_expr(c, ctx)?;
            let prop = disasm_expr(c, ctx)?;
            let val  = disasm_expr(c, ctx)?;
            format!("{}.Find({}, {})", arr, prop, val)
        }
        ExprToken::DynArraySort => {
            let arr = disasm_expr(c, ctx)?;
            let cmp = disasm_expr(c, ctx)?;
            format!("{}.Sort({})", arr, cmp)
        }
        ExprToken::DynArrayIterator => {
            let arr      = disasm_expr(c, ctx)?;
            let iter_var = disasm_expr(c, ctx)?;
            let _skip    = c.read_u16::<LittleEndian>()?;
            format!("foreach {}({}) ", arr, iter_var)
        }
        ExprToken::Iterator => {
            let e     = disasm_expr(c, ctx)?;
            let _skip = c.read_u16::<LittleEndian>()?;
            format!("foreach {} ", e)
        }

        ExprToken::DynamicCast | ExprToken::MetaCast | ExprToken::InterfaceCast => {
            let class = c.read_i32::<LittleEndian>()?;
            let inner = disasm_expr(c, ctx)?;
            format!("{}({})", resolve_obj_ref(class, ctx.pak), inner)
        }
        ExprToken::PrimitiveCast => {
            let cast_byte = c.read_u8()?;
            let inner     = disasm_expr(c, ctx)?;
            format!("{}({})", cast_name(cast_byte), inner)
        }

        ExprToken::New => {
            let outer = disasm_expr(c, ctx)?;
            let name  = disasm_expr(c, ctx)?;
            let flags = disasm_expr(c, ctx)?;
            let class = disasm_expr(c, ctx)?;
            let arch  = disasm_expr(c, ctx)?;
            format!("new({}, {}, {}) {}({})", outer, name, flags, class, arch)
        }

        ExprToken::StructCmpEq | ExprToken::StructCmpNe => {
            let strct = c.read_i32::<LittleEndian>()?;
            let lhs   = disasm_expr(c, ctx)?;
            let rhs   = disasm_expr(c, ctx)?;
            let op    = if tok == ExprToken::StructCmpEq { "==" } else { "!=" };
            format!("({} {} {}) /*struct {}*/", lhs, op, rhs, resolve_obj_ref(strct, ctx.pak))
        }
        ExprToken::EqualEqual_DelDel | ExprToken::EqualEqual_DelFunc => {
            let lhs = disasm_expr(c, ctx)?;
            let rhs = disasm_expr(c, ctx)?;
            format!("({} == {})", lhs, rhs)
        }
        ExprToken::NotEqual_DelDel | ExprToken::NotEqual_DelFunc => {
            let lhs = disasm_expr(c, ctx)?;
            let rhs = disasm_expr(c, ctx)?;
            format!("({} != {})", lhs, rhs)
        }

        ExprToken::Conditional => {
            let cond   = disasm_expr(c, ctx)?;
            let _skip1 = c.read_u16::<LittleEndian>()?;
            let then_e = disasm_expr(c, ctx)?;
            let _skip2 = c.read_u16::<LittleEndian>()?;
            let else_e = disasm_expr(c, ctx)?;
            format!("({} ? {} : {})", cond, then_e, else_e)
        }

        ExprToken::Skip => {
            let _sz = c.read_u16::<LittleEndian>()?;
            disasm_expr(c, ctx)?
        }
        ExprToken::DefaultParmValue => {
            let _sz = c.read_u16::<LittleEndian>()?;
            let val = disasm_expr(c, ctx)?;
            format!("/*default={}*/", val)
        }

        ExprToken::DebugInfo => {
            // EX_DebugInfo: version(i32) line(i32) col(i32) opcode(u8)
            let _ver    = c.read_i32::<LittleEndian>()?;
            let _line   = c.read_i32::<LittleEndian>()?;
            let _col    = c.read_i32::<LittleEndian>()?;
            let _opcode = c.read_u8()?;
            String::new() // callers skip empty results
        }

        ExprToken::ExtendedNative => {
            // 0x60..=0x6F: native index = ((low nibble) << 8) | next_byte
            let low  = (raw & 0x0F) as u16;
            let high = c.read_u8()? as u16;
            let idx  = (low << 8) | high;
            let args = disasm_params(c, ctx)?;
            format!("Native_{}({})", idx, args)
        }
        ExprToken::Unknown if raw >= 0x70 => {
            // 0x70..=0xFF: directly encoded native index
            let args = disasm_params(c, ctx)?;
            format!("Native_{}({})", raw as u16, args)
        }

        _ => format!("/*UNKNOWN_OPCODE 0x{:02X} @ 0x{:04X}*/", raw, pos),
    };

    Ok(expr)
}

fn disasm_params(c: &mut Cursor<&[u8]>, ctx: &DisasmCtx) -> Result<String> {
    let mut args = Vec::new();
    loop {
        let peek = c.read_u8()?;
        if peek == ExprToken::EndFunctionParms as u8 { break; }
        c.seek(SeekFrom::Current(-1))?;
        let arg = disasm_expr(c, ctx)?;
        if !arg.is_empty() && !arg.starts_with("/*") {
            args.push(arg);
        }
    }
    Ok(args.join(", "))
}

// ── Script-array extraction ───────────────────────────────────────────────────

/// Attempt to read a `TArray<BYTE>` Script blob starting at the cursor's
/// current position.  Returns `Some(bytes)` if the size prefix looks
/// plausible and the slice ends with `EX_EndOfScript` (0x53).
fn try_read_script_array(c: &mut Cursor<&[u8]>, blob_len: usize) -> Option<Vec<u8>> {
    let saved = c.position();
    let sz = c.read_i32::<LittleEndian>().ok()? as usize;
    if sz == 0 || sz > 0x4_0000 { // sanity cap: 256 KiB
        c.set_position(saved);
        return None;
    }
    let pos_after = c.position() as usize;
    if pos_after + sz > blob_len {
        c.set_position(saved);
        return None;
    }
    let mut buf = vec![0u8; sz];
    c.read_exact(&mut buf).ok()?;
    // Must contain EX_EndOfScript somewhere near the end
    if buf.iter().rev().take(8).any(|&b| b == 0x53) {
        Some(buf)
    } else {
        c.set_position(saved);
        None
    }
}

/// Extract the `Script TArray<BYTE>` from a raw serialized `UFunction` export
/// blob.
///
/// UE3 serial layout for a `UFunction`:
/// ```
/// [net_index: i32]                       <- UObject prefix (pos 0)
/// [tagged properties...]                 <- UObject::Serialize
///     "None" terminator
/// [Next:        i32]                     <- UField::Serialize
/// [SuperField:  i32]                     <- UStruct::Serialize
/// [Children:    i32]                     <- UStruct::Serialize
/// [Script count: i32, Script bytes: ...]  <- TArray<BYTE>
/// [... UFunction-specific fields ...]
/// ```
///
/// We skip the properties via lightweight name-index scanning (not full prop
/// parsing, which requires a mutable `Cursor<&Vec<u8>>`), then probe up to
/// four i32 slots before the Script count to handle version differences.
pub fn extract_script_from_export_blob(blob: &[u8], pak: &UPKPak) -> Option<Vec<u8>> {
    if blob.len() < 8 { return None; }

    let mut c = Cursor::new(blob);

    // ── 1. Skip the net-index i32 at position 0 ──────────────────────────────
    c.seek(SeekFrom::Start(4)).ok()?;

    // ── 2. Skip tagged properties until "None" ────────────────────────────────
    // Each property starts with an FName (8 bytes: name_index i32 + number i32).
    // We read name indices and stop when we see "None" (or 0 in many files).
    // Rather than a full property parser we use a conservative skip loop:
    // read an FName, look up the name; if "None" (or invalid), we're done.
    // For each real property we must skip past its header + payload — since we
    // don't have full type info, we fall through to the heuristic if this step
    // confuses itself.
    let props_end = skip_tagged_properties(&mut c, pak)?;
    c.set_position(props_end);

    // ── 3. Skip UField::Next + UStruct::SuperField + Children ────────────────
    // Probe 0-3 extra i32 skips to accommodate version differences.
    for extra_skips in 0u64..=4 {
        let probe_pos = props_end + extra_skips * 4;
        if probe_pos as usize >= blob.len() { break; }
        let mut probe = Cursor::new(blob);
        probe.set_position(probe_pos);
        if let Some(script) = try_read_script_array(&mut probe, blob.len()) {
            return Some(script);
        }
    }

    // ── 4. Fallback: byte-scan for a plausible TArray ────────────────────────
    heuristic_script_scan(blob)
}

/// Walk tagged properties by reading FName headers (8 bytes each) and using
/// the size field in the property header to jump forward.  Returns the stream
/// position immediately after the terminal "None" name.
///
/// Property header layout (after the FName name, which we already read):
///   FName type (8 bytes)
///   i32  size
///   i32  array_index
///   [optional 8-byte enum FName for ByteProperty]
///   <size bytes of value>
///
/// We only need the name to detect "None"; for others we read the full header
/// to skip the payload correctly.
fn skip_tagged_properties(c: &mut Cursor<&[u8]>, pak: &UPKPak) -> Option<u64> {
    loop {
        let name_idx  = c.read_i32::<LittleEndian>().ok()? as usize;
        let _name_num = c.read_i32::<LittleEndian>().ok()?;

        let name = pak.name_table.get(name_idx).map(|s| s.as_str()).unwrap_or("");

        if name.is_empty() || name == "None" {
            return Some(c.position());
        }

        // Read the rest of the property header to advance past the value.
        let _type_idx  = c.read_i32::<LittleEndian>().ok()?;
        let _type_num  = c.read_i32::<LittleEndian>().ok()?;
        let size       = c.read_i32::<LittleEndian>().ok()? as i64;
        let _arr_idx   = c.read_i32::<LittleEndian>().ok()?;

        if size < 0 || size > 0x10_0000 { return None; } // sanity

        // ByteProperty has an extra 8-byte enum FName before the value
        // We can't know the type name easily here without another name lookup,
        // so we optimistically skip size bytes; if size is exactly 1, it's a
        // plain byte and no enum name is present (enum values have size 8).
        c.seek(SeekFrom::Current(size)).ok()?;
    }
}

/// Last-resort: scan for any i32 that looks like a sane Script size and is
/// followed by bytes whose last few contain EX_EndOfScript (0x53).
fn heuristic_script_scan(blob: &[u8]) -> Option<Vec<u8>> {
    for i in 4..blob.len().saturating_sub(4) {
        let sz = i32::from_le_bytes(blob[i..i+4].try_into().ok()?) as usize;
        if sz < 4 || sz > 0x4_0000 { continue; }
        let end = i + 4 + sz;
        if end > blob.len() { continue; }
        let candidate = &blob[i+4..end];
        if candidate.iter().rev().take(8).any(|&b| b == 0x53) {
            return Some(candidate.to_vec());
        }
    }
    None
}

// ── Top-level API ─────────────────────────────────────────────────────────────

/// Disassemble a raw `Script` byte array (from a UFunction export).
/// Returns a `Vec` of `(bytecode_offset, statement_string)`.
pub fn disasm_function(script: &[u8], pak: &UPKPak) -> Vec<(usize, String)> {
    let mut c   = Cursor::new(script);
    let ctx     = DisasmCtx::new(pak);
    let mut out = Vec::new();

    while (c.position() as usize) < script.len() {
        let pos = c.position() as usize;

        match c.read_u8() {
            Ok(b) if b == ExprToken::EndOfScript as u8 => break,
            Ok(_)  => { c.seek(SeekFrom::Current(-1)).ok(); }
            Err(_) => break,
        }

        match disasm_expr(&mut c, &ctx) {
            Ok(s) if !s.is_empty() => out.push((pos, s)),
            Ok(_)  => {}
            Err(e) => {
                out.push((pos, format!("/*ERROR @ 0x{:04X}: {}*/", pos, e)));
                break;
            }
        }
    }

    out
}

/// Format the output of `disasm_function` as a human-readable string.
pub fn print_disasm(stmts: &[(usize, String)]) -> String {
    let mut out = String::new();
    for (off, s) in stmts {
        out.push_str(&format!("/* 0x{:04X} */  {};\n", off, s));
    }
    out
}
