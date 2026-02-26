// src/scriptcompiler.rs
// UnrealScript bytecode compiler for UE3.
//
// This is a direct bytecode emitter — it reads a minimal assembly-like text
// format and emits raw bytes for use with MakeScriptPatch.
//
// Format example:
//   // comment
//   InstanceVariable  WeaponDamage
//   IntConst          42
//   Let
//   Return Nothing
//
// For real use you'd want a full .uc parser; this gives you a working
// foundation to extend.

use std::collections::HashMap;
use std::io::{Result, Write};
use byteorder::{LittleEndian, WriteBytesExt};
use crate::upkreader::UPKPak;
use crate::scriptdisasm::ExprToken;

// ── Name resolution helpers ───────────────────────────────────────────────────

/// Build a name→index map from the package name table.
pub fn build_name_map(pak: &UPKPak) -> HashMap<String, usize> {
    pak.name_table.iter().enumerate()
        .map(|(i, n)| (n.clone(), i))
        .collect()
}

/// Build an object name→export-index map (1-based, as used in bytecode).
pub fn build_export_map(pak: &UPKPak) -> HashMap<String, i32> {
    pak.export_table.iter().enumerate()
        .map(|(i, e)| {
            let name = pak.name_table
                .get(e.object_name.name_index as usize)
                .map(|n| n.clone())
                .unwrap_or_default();
            (name, (i + 1) as i32)
        })
        .collect()
}

/// Build an object name→import-index map (negative, as used in bytecode).
pub fn build_import_map(pak: &UPKPak) -> HashMap<String, i32> {
    pak.import_table.iter().enumerate()
        .map(|(i, im)| {
            let name = pak.name_table
                .get(im.object_name.name_index as usize)
                .map(|n| n.clone())
                .unwrap_or_default();
            (name, -((i + 1) as i32))
        })
        .collect()
}

fn resolve_obj(name: &str, exports: &HashMap<String, i32>, imports: &HashMap<String, i32>) -> i32 {
    if name == "None" || name == "null" { return 0; }
    if let Some(&v) = exports.get(name) { return v; }
    if let Some(&v) = imports.get(name) { return v; }
    eprintln!("WARNING: unresolved object '{}', emitting 0", name);
    0
}

// ── Emit helpers ──────────────────────────────────────────────────────────────

fn emit_obj<W: Write>(w: &mut W, name: &str, exp: &HashMap<String, i32>, imp: &HashMap<String, i32>) -> Result<()> {
    let idx = resolve_obj(name, exp, imp);
    w.write_i32::<LittleEndian>(idx)
}

fn emit_fname<W: Write>(w: &mut W, name: &str, names: &HashMap<String, usize>) -> Result<()> {
    let idx = names.get(name).copied().unwrap_or_else(|| {
        eprintln!("WARNING: name '{}' not in name table, emitting 0", name);
        0
    });
    w.write_i32::<LittleEndian>(idx as i32)?;
    w.write_i32::<LittleEndian>(0) // name_instance = 0
}

// ── Token descriptor: how to parse the rest of a token's arguments ────────────

#[derive(Debug, Clone)]
enum Arg {
    ObjRef,          // i32 package index resolved from a name
    FName,           // 8 bytes (i32 name_idx, i32 instance)
    U8,              // 1 byte literal
    U16,             // 2 bytes
    I32,             // 4 bytes literal
    F32,             // 4 bytes float
    CString,         // null-terminated ASCII
    UString,         // null-terminated UTF-16LE (each char 2 bytes)
    SubExpr,         // a nested expression (recursive compile)
    Params,          // zero or more sub-expressions until EndFunctionParms
}

// ── AST node (very minimal) ───────────────────────────────────────────────────

/// A compiled instruction ready to emit.
#[derive(Debug, Clone)]
pub enum Insn {
    Raw(Vec<u8>),          // pre-built byte sequence
    JumpForward(u16),      // filled-in jump offset (label resolution TODO)
}

// ── Main compiler context ─────────────────────────────────────────────────────

pub struct Compiler<'a> {
    pak: &'a UPKPak,
    names: HashMap<String, usize>,
    exports: HashMap<String, i32>,
    imports: HashMap<String, i32>,
    /// label name → byte offset in output
    pub labels: HashMap<String, u16>,
    /// (output_offset_of_word, label_name) for back-patching jump offsets
    pub fixups: Vec<(usize, String)>,
    pub out: Vec<u8>,
}

impl<'a> Compiler<'a> {
    pub fn new(pak: &'a UPKPak) -> Self {
        let names = build_name_map(pak);
        let exports = build_export_map(pak);
        let imports = build_import_map(pak);
        Compiler { pak, names, exports, imports, labels: HashMap::new(), fixups: Vec::new(), out: Vec::new() }
    }

    fn pos(&self) -> usize { self.out.len() }

    fn emit_u8(&mut self, b: u8) { self.out.push(b); }

    fn emit_i32(&mut self, v: i32) {
        self.out.extend_from_slice(&v.to_le_bytes());
    }

    fn emit_u16(&mut self, v: u16) {
        self.out.extend_from_slice(&v.to_le_bytes());
    }

    fn emit_f32(&mut self, v: f32) {
        self.out.extend_from_slice(&v.to_le_bytes());
    }

    fn emit_obj(&mut self, name: &str) {
        let idx = resolve_obj(name, &self.exports, &self.imports);
        self.emit_i32(idx);
    }

    fn emit_fname(&mut self, name: &str) {
        let idx = self.names.get(name).copied().unwrap_or(0);
        self.emit_i32(idx as i32);
        self.emit_i32(0);
    }

    fn emit_cstring(&mut self, s: &str) {
        self.out.extend_from_slice(s.as_bytes());
        self.out.push(0);
    }

    fn emit_ustring(&mut self, s: &str) {
        for ch in s.encode_utf16() {
            self.out.extend_from_slice(&ch.to_le_bytes());
        }
        self.out.extend_from_slice(&[0u8, 0u8]); // null terminator
    }

    /// Reserve a u16 slot for a jump target; returns the offset of the slot.
    fn reserve_u16(&mut self) -> usize {
        let pos = self.pos();
        self.emit_u16(0xDEAD);
        pos
    }

    /// Back-patch a u16 at `slot` with `value`.
    pub fn patch_u16(&mut self, slot: usize, value: u16) {
        self.out[slot..slot+2].copy_from_slice(&value.to_le_bytes());
    }

    /// Compile a token line.  `line` is like:
    ///   "IntConst 42"
    ///   "VirtualFunction FunctionName arg1 arg2 EndFunctionParms"
    ///   "Jump @MyLabel"
    ///   "@MyLabel"          (label definition)
    pub fn compile_line(&mut self, line: &str) -> Result<()> {
        let line = line.trim();
        if line.is_empty() || line.starts_with("//") || line.starts_with('#') {
            return Ok(());
        }

        // Label definition: "@LabelName"
        if line.starts_with('@') {
            let label = line[1..].trim().to_string();
            self.labels.insert(label, self.pos() as u16);
            return Ok(());
        }

        let mut parts = line.split_whitespace();
        let mnemonic = parts.next().unwrap_or("");
        let args: Vec<&str> = parts.collect();

        match mnemonic {
            // ── Variables ────────────────────────────────────────────────────
            "LocalVariable" | "LocalVar" => {
                self.emit_u8(ExprToken::LocalVariable as u8);
                self.emit_obj(args.first().copied().unwrap_or("None"));
            }
            "InstanceVariable" | "InstanceVar" => {
                self.emit_u8(ExprToken::InstanceVariable as u8);
                self.emit_obj(args.first().copied().unwrap_or("None"));
            }
            "DefaultVariable" | "DefaultVar" => {
                self.emit_u8(ExprToken::DefaultVariable as u8);
                self.emit_obj(args.first().copied().unwrap_or("None"));
            }

            // ── Control flow ─────────────────────────────────────────────────
            "Return" => {
                self.emit_u8(ExprToken::Return as u8);
                // next arg should be the return value expression or "Nothing"
                if args.first().copied() == Some("Nothing") {
                    self.emit_u8(ExprToken::Nothing as u8);
                }
            }
            "ReturnNothing" => { self.emit_u8(ExprToken::ReturnNothing as u8); }
            "Stop"          => { self.emit_u8(ExprToken::Stop as u8); }
            "Nothing"       => { self.emit_u8(ExprToken::Nothing as u8); }
            "EndOfScript"   => { self.emit_u8(ExprToken::EndOfScript as u8); }
            "EndFunctionParms" => { self.emit_u8(ExprToken::EndFunctionParms as u8); }
            "Self"          => { self.emit_u8(ExprToken::Self_ as u8); }
            "NoObject"      => { self.emit_u8(ExprToken::NoObject as u8); }
            "True"          => { self.emit_u8(ExprToken::True as u8); }
            "False"         => { self.emit_u8(ExprToken::False as u8); }
            "IntZero"       => { self.emit_u8(ExprToken::IntZero as u8); }
            "IntOne"        => { self.emit_u8(ExprToken::IntOne as u8); }
            "IteratorNext"  => { self.emit_u8(ExprToken::IteratorNext as u8); }
            "IteratorPop"   => { self.emit_u8(ExprToken::IteratorPop as u8); }

            "Jump" => {
                self.emit_u8(ExprToken::Jump as u8);
                // If arg starts with '@', it's a forward label ref
                let target = args.first().copied().unwrap_or("0");
                if let Some(lbl) = target.strip_prefix('@') {
                    if let Some(&off) = self.labels.get(lbl) {
                        self.emit_u16(off);
                    } else {
                        // forward reference — reserve and fixup later
                        let slot = self.reserve_u16();
                        self.fixups.push((slot, lbl.to_string()));
                    }
                } else {
                    let off = u16::from_str_radix(target.trim_start_matches("0x"), 16)
                        .or_else(|_| target.parse::<u16>())
                        .unwrap_or(0);
                    self.emit_u16(off);
                }
            }
            "JumpIfNot" => {
                self.emit_u8(ExprToken::JumpIfNot as u8);
                let target = args.first().copied().unwrap_or("0");
                if let Some(lbl) = target.strip_prefix('@') {
                    if let Some(&off) = self.labels.get(lbl) {
                        self.emit_u16(off);
                    } else {
                        let slot = self.reserve_u16();
                        self.fixups.push((slot, lbl.to_string()));
                    }
                } else {
                    let off: u16 = target.parse().unwrap_or(0);
                    self.emit_u16(off);
                }
                // Caller must then emit the condition expression on the next line
            }

            "Let" => {
                self.emit_u8(ExprToken::Let as u8);
                // lhs/rhs come from subsequent lines
            }
            "LetBool" => {
                self.emit_u8(ExprToken::LetBool as u8);
            }

            // ── Constants ─────────────────────────────────────────────────────
            "IntConst" => {
                let v: i32 = args.first().copied().unwrap_or("0")
                    .parse().unwrap_or(0);
                if v == 0 { self.emit_u8(ExprToken::IntZero as u8); }
                else if v == 1 { self.emit_u8(ExprToken::IntOne as u8); }
                else if (0..=255).contains(&v) {
                    self.emit_u8(ExprToken::IntConstByte as u8);
                    self.emit_u8(v as u8);
                } else {
                    self.emit_u8(ExprToken::IntConst as u8);
                    self.emit_i32(v);
                }
            }
            "FloatConst" => {
                self.emit_u8(ExprToken::FloatConst as u8);
                let v: f32 = args.first().copied().unwrap_or("0")
                    .trim_end_matches('f').parse().unwrap_or(0.0);
                self.emit_f32(v);
            }
            "ByteConst" => {
                self.emit_u8(ExprToken::ByteConst as u8);
                let v: u8 = args.first().copied().unwrap_or("0")
                    .parse().unwrap_or(0);
                self.emit_u8(v);
            }
            "StringConst" | "StrConst" => {
                self.emit_u8(ExprToken::StringConst as u8);
                // Join remaining args for strings with spaces; strip quotes
                let s = args.join(" ").trim_matches('"').to_string();
                self.emit_cstring(&s);
            }
            "NameConst" => {
                self.emit_u8(ExprToken::NameConst as u8);
                let name = args.first().copied().unwrap_or("None")
                    .trim_matches('\'');
                self.emit_fname(name);
            }
            "ObjectConst" => {
                // ObjectConst <class_name> <obj_name>
                self.emit_u8(ExprToken::ObjectConst as u8);
                let obj   = args.first().copied().unwrap_or("None");
                let class = args.get(1).copied().unwrap_or("None");
                self.emit_obj(obj);
                self.emit_obj(class);
            }
            "VectorConst" => {
                // VectorConst X Y Z
                self.emit_u8(ExprToken::VectorConst as u8);
                let x: f32 = args.first().copied().unwrap_or("0").parse().unwrap_or(0.0);
                let y: f32 = args.get(1).copied().unwrap_or("0").parse().unwrap_or(0.0);
                let z: f32 = args.get(2).copied().unwrap_or("0").parse().unwrap_or(0.0);
                self.emit_f32(x); self.emit_f32(y); self.emit_f32(z);
            }
            "RotationConst" => {
                // RotationConst Pitch Yaw Roll
                self.emit_u8(ExprToken::RotationConst as u8);
                let p: i32 = args.first().copied().unwrap_or("0").parse().unwrap_or(0);
                let y: i32 = args.get(1).copied().unwrap_or("0").parse().unwrap_or(0);
                let r: i32 = args.get(2).copied().unwrap_or("0").parse().unwrap_or(0);
                self.emit_i32(p); self.emit_i32(y); self.emit_i32(r);
            }

            // ── Function calls ────────────────────────────────────────────────
            "VirtualFunction" => {
                self.emit_u8(ExprToken::VirtualFunction as u8);
                let name = args.first().copied().unwrap_or("None");
                self.emit_fname(name);
                // params follow as subsequent lines until EndFunctionParms
            }
            "FinalFunction" => {
                self.emit_u8(ExprToken::FinalFunction as u8);
                let obj = args.first().copied().unwrap_or("None");
                self.emit_obj(obj);
            }
            "GlobalFunction" => {
                self.emit_u8(ExprToken::GlobalFunction as u8);
                let name = args.first().copied().unwrap_or("None");
                self.emit_fname(name);
            }

            // ── Dynamic array ─────────────────────────────────────────────────
            "DynArrayLength" => { self.emit_u8(ExprToken::DynArrayLength as u8); }
            "DynArrayElement" => { self.emit_u8(ExprToken::DynArrayElement as u8); }
            "DynArrayAdd"     => { self.emit_u8(ExprToken::DynArrayAdd as u8); }
            "DynArrayAddItem" => { self.emit_u8(ExprToken::DynArrayAddItem as u8); }
            "DynArrayRemove"  => { self.emit_u8(ExprToken::DynArrayRemove as u8); }
            "DynArrayRemoveItem" => { self.emit_u8(ExprToken::DynArrayRemoveItem as u8); }
            "DynArrayInsert"  => { self.emit_u8(ExprToken::DynArrayInsert as u8); }
            "DynArrayInsertItem" => { self.emit_u8(ExprToken::DynArrayInsertItem as u8); }
            "DynArrayFind"    => { self.emit_u8(ExprToken::DynArrayFind as u8); }
            "DynArraySort"    => { self.emit_u8(ExprToken::DynArraySort as u8); }

            // ── Casts ─────────────────────────────────────────────────────────
            "DynamicCast" | "Cast" => {
                self.emit_u8(ExprToken::DynamicCast as u8);
                let class = args.first().copied().unwrap_or("None");
                self.emit_obj(class);
            }
            "PrimitiveCast" => {
                // PrimitiveCast ByteToInt  (or the hex byte value)
                self.emit_u8(ExprToken::PrimitiveCast as u8);
                let cast_arg = args.first().copied().unwrap_or("0x3A");
                let cast_byte = primitive_cast_byte(cast_arg);
                self.emit_u8(cast_byte);
            }

            // ── Raw byte injection (escape hatch) ─────────────────────────────
            "RawByte" | "DB" => {
                for a in &args {
                    let b = u8::from_str_radix(a.trim_start_matches("0x"), 16)
                        .or_else(|_| a.parse::<u8>())
                        .unwrap_or(0);
                    self.emit_u8(b);
                }
            }
            "RawI32" | "DW" => {
                for a in &args {
                    let v: i32 = a.parse().unwrap_or(0);
                    self.emit_i32(v);
                }
            }

            unknown => {
                eprintln!("WARNING: unknown mnemonic '{}', skipping", unknown);
            }
        }

        Ok(())
    }

    /// Apply all forward-reference fixups.  Call after all lines are compiled.
    pub fn apply_fixups(&mut self) {
        let fixups = std::mem::take(&mut self.fixups);
        for (slot, lbl) in fixups {
            if let Some(&off) = self.labels.get(&lbl) {
                self.patch_u16(slot, off);
            } else {
                eprintln!("WARNING: label '@{}' never defined", lbl);
            }
        }
    }

    /// Compile a multi-line script text, returning the bytecode.
    pub fn compile_text(&mut self, text: &str) -> Result<Vec<u8>> {
        for line in text.lines() {
            self.compile_line(line)?;
        }
        self.apply_fixups();
        Ok(self.out.clone())
    }
}

fn primitive_cast_byte(name: &str) -> u8 {
    match name {
        "InterfaceToObject" => 0x36,
        "InterfaceToString" => 0x37,
        "InterfaceToBool"   => 0x38,
        "RotatorToVector"   => 0x39,
        "ByteToInt"         => 0x3A,
        "ByteToBool"        => 0x3B,
        "ByteToFloat"       => 0x3C,
        "IntToByte"         => 0x3D,
        "IntToBool"         => 0x3E,
        "IntToFloat"        => 0x3F,
        "BoolToByte"        => 0x40,
        "BoolToInt"         => 0x41,
        "BoolToFloat"       => 0x42,
        "FloatToByte"       => 0x43,
        "FloatToInt"        => 0x44,
        "FloatToBool"       => 0x45,
        "ObjectToInterface" => 0x46,
        "ObjectToBool"      => 0x47,
        "NameToBool"        => 0x48,
        "StringToByte"      => 0x49,
        "StringToInt"       => 0x4A,
        "StringToBool"      => 0x4B,
        "StringToFloat"     => 0x4C,
        "StringToVector"    => 0x4D,
        "StringToRotator"   => 0x4E,
        "VectorToBool"      => 0x4F,
        "VectorToRotator"   => 0x50,
        "RotatorToBool"     => 0x51,
        "ByteToString"      => 0x52,
        "IntToString"       => 0x53,
        "BoolToString"      => 0x54,
        "FloatToString"     => 0x55,
        "ObjectToString"    => 0x56,
        "NameToString"      => 0x57,
        "VectorToString"    => 0x58,
        "RotatorToString"   => 0x59,
        "DelegateToString"  => 0x5A,
        "StringToName"      => 0x60,
        other => u8::from_str_radix(other.trim_start_matches("0x"), 16).unwrap_or(0),
    }
}
