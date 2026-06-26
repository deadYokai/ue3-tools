use std::fmt;

#[derive(Debug, Clone, PartialEq)]
pub enum PseudoValue {
    Null,
    Num(String),
    Bool(bool),
    Str(String),
    Name(String),
    Enum(String),
    Ref(String),
    Array(Vec<PseudoValue>),
    Object(Vec<(String, PseudoValue)>),
    Opaque(String),
}

#[derive(Debug, Clone, Default)]
pub struct PseudoFile {
    pub pkg_stem: Option<String>,
    pub p_ver: Option<i16>,
    pub export_index: Option<i32>,
    pub full_path: Option<String>,
    pub net_index: Option<i32>,

    pub is_definition: bool,
    pub class_name: String,
    pub object_name: String,
    pub fields: Vec<(String, PseudoValue)>,
    pub native_fields: Vec<(String, PseudoValue)>,
    pub sidecars: Vec<String>,
}

#[derive(Debug)]
pub struct ParseError {
    pub offset: usize,
    pub msg: String,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, ".uo parse error at byte {}: {}", self.offset, self.msg)
    }
}
impl std::error::Error for ParseError {}

impl From<ParseError> for std::io::Error {
    fn from(e: ParseError) -> Self {
        std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string())
    }
}

type PResult<T> = Result<T, ParseError>;

pub fn parse(src: &str) -> PResult<PseudoFile> {
    let mut p = Parser::new(src);
    p.parse_file()
}

fn extract_quoted(s: &str) -> Option<String> {
    let start = s.find('"')? + 1;
    let rest = &s[start..];
    let end = rest.find('"')?;
    Some(rest[..end].to_string())
}

const DEF_KEYWORDS: &[&str] = &[
    "class",
    "struct",
    "enum",
    "const",
    "var",
    "function",
    "state",
    "immutable",
    "immutablewhencooked",
];

struct Parser<'a> {
    s: &'a [u8],
    src: &'a str,
    i: usize,
    native_fields: Vec<(String, PseudoValue)>,
    sidecars: Vec<String>,
}

impl<'a> Parser<'a> {
    fn new(src: &'a str) -> Self {
        Parser {
            s: src.as_bytes(),
            src,
            i: 0,
            native_fields: Vec::new(),
            sidecars: Vec::new(),
        }
    }

    fn err<T>(&self, msg: impl Into<String>) -> PResult<T> {
        Err(ParseError {
            offset: self.i,
            msg: msg.into(),
        })
    }

    fn peek(&self) -> Option<u8> {
        self.s.get(self.i).copied()
    }

    fn bump(&mut self) -> Option<u8> {
        let c = self.peek();
        if c.is_some() {
            self.i += 1;
        }
        c
    }

    fn starts_with(&self, kw: &str) -> bool {
        self.src[self.i..].starts_with(kw)
    }

    fn skip_ws(&mut self) {
        while let Some(c) = self.peek() {
            if c == b' ' || c == b'\t' || c == b'\r' || c == b'\n' {
                self.i += 1;
            } else {
                break;
            }
        }
    }

    fn skip_trivia(&mut self) {
        loop {
            self.skip_ws();
            if self.starts_with("//") {
                self.skip_to_eol();
            } else {
                break;
            }
        }
    }

    fn skip_trivia_marking_native(&mut self, in_native: &mut bool) {
        loop {
            self.skip_ws();
            if self.starts_with("//") {
                let start = self.i;
                self.skip_to_eol();
                if self.src[start..self.i].contains("native-serialized") {
                    *in_native = true;
                }
            } else {
                break;
            }
        }
    }

    fn skip_to_eol(&mut self) {
        while let Some(c) = self.peek() {
            self.i += 1;
            if c == b'\n' {
                break;
            }
        }
    }

    fn rest_of_line(&mut self) -> &'a str {
        let start = self.i;
        while let Some(c) = self.peek() {
            if c == b'\n' {
                break;
            }
            self.i += 1;
        }
        self.src[start..self.i].trim_end()
    }

    fn parse_file(&mut self) -> PResult<PseudoFile> {
        let mut out = PseudoFile::default();

        loop {
            self.skip_ws();
            if self.starts_with("//") {
                self.i += 2;
                let line = self.rest_of_line().trim().to_string();
                self.parse_header_line(&line, &mut out);
            } else {
                break;
            }
        }

        self.skip_trivia();
        if self.peek().is_none() {
            return self.err("unexpected end of file before object body");
        }

        let first = self.read_ident();
        if first.is_empty() {
            return self.err("expected class/object declaration");
        }
        if DEF_KEYWORDS.contains(&first.as_str()) {
            out.is_definition = true;
            return Ok(out);
        }

        out.class_name = first;
        self.skip_trivia();
        out.object_name = self.read_ident();
        self.skip_trivia();
        if self.peek() != Some(b'{') {
            return self.err("expected '{' after object name");
        }
        out.fields = self.parse_object_fields()?;
        out.native_fields = std::mem::take(&mut self.native_fields);
        out.sidecars = std::mem::take(&mut self.sidecars);
        Ok(out)
    }

    fn parse_header_line(&self, line: &str, out: &mut PseudoFile) {
        if let Some(rest) = line.strip_prefix("path:") {
            out.full_path = Some(rest.trim().to_string());
            return;
        }
        if let Some(rest) = line.strip_prefix("net_index:") {
            out.net_index = rest.trim().parse().ok();
            return;
        }
        for tok in line.split_whitespace() {
            if let Some(v) = tok.strip_prefix("pkg=") {
                out.pkg_stem = Some(v.trim_end_matches(".upk").to_string());
            } else if let Some(v) = tok.strip_prefix("p_ver=") {
                out.p_ver = v.parse().ok();
            } else if let Some(v) = tok.strip_prefix("export=#") {
                out.export_index = v.parse().ok();
            }
        }
    }

    fn parse_object_fields(&mut self) -> PResult<Vec<(String, PseudoValue)>> {
        debug_assert_eq!(self.peek(), Some(b'{'));
        self.i += 1; // consume '{'
        let mut fields = Vec::new();
        let mut in_native = false;
        loop {
            self.skip_trivia_marking_native(&mut in_native);
            match self.peek() {
                None => return self.err("unterminated '{' object"),
                Some(b'}') => {
                    self.i += 1;
                    break;
                }
                Some(b'@') => {
                    let label = self.rest_of_line().to_string();
                    if label.starts_with("@sidecar") {
                        if let Some(name) = extract_quoted(&label) {
                            self.sidecars.push(name);
                        }
                        continue;
                    }
                    if label.starts_with("@native") {
                        self.skip_balanced_braces_after_line()?;
                    }
                    fields.push(("@native".to_string(), PseudoValue::Opaque(label)));
                    continue;
                }
                _ => {}
            }
            let name = self.read_ident();
            if name.is_empty() {
                return self.err("expected field name");
            }
            self.skip_trivia();
            if self.peek() != Some(b'=') {
                return self.err(format!("expected '=' after field '{name}'"));
            }
            self.i += 1;
            self.skip_trivia();
            let val = self.parse_value()?;
            if in_native {
                self.native_fields.push((name, val));
            } else {
                fields.push((name, val));
            }
            self.skip_ws();
            if self.peek() == Some(b',') {
                self.i += 1;
            }
        }
        Ok(fields)
    }

    fn skip_balanced_braces_after_line(&mut self) -> PResult<()> {
        let mut depth = 1i32;
        while let Some(c) = self.bump() {
            match c {
                b'{' => depth += 1,
                b'}' => {
                    depth -= 1;
                    if depth == 0 {
                        return Ok(());
                    }
                }
                b'"' => self.consume_quoted(b'"'),
                b'\'' => self.consume_quoted(b'\''),
                _ => {}
            }
        }
        self.err("unterminated @native block")
    }

    fn consume_quoted(&mut self, q: u8) {
        while let Some(c) = self.bump() {
            if c == b'\\' {
                self.bump();
            } else if c == q {
                break;
            }
        }
    }

    fn parse_value(&mut self) -> PResult<PseudoValue> {
        self.skip_trivia();
        match self.peek() {
            None => self.err("expected value"),
            Some(b'{') => {
                let fields = self.parse_object_fields()?;
                Ok(PseudoValue::Object(fields))
            }
            Some(b'[') => self.parse_array(),
            Some(b'"') => Ok(PseudoValue::Str(self.parse_quoted_string()?)),
            Some(b'\'') => Ok(PseudoValue::Name(self.parse_quoted_name()?)),
            Some(b'&') => {
                self.i += 1;
                Ok(PseudoValue::Ref(self.read_ref_label()))
            }
            Some(b'@') => Ok(PseudoValue::Opaque(self.rest_of_line().to_string())),
            Some(c) if c == b'-' || c.is_ascii_digit() => self.parse_number(),
            Some(_) => {
                let w = self.read_bareword();
                match w.as_str() {
                    "None" => Ok(PseudoValue::Null),
                    "true" => Ok(PseudoValue::Bool(true)),
                    "false" => Ok(PseudoValue::Bool(false)),
                    "" => self.err("expected value"),
                    _ => Ok(PseudoValue::Enum(w)),
                }
            }
        }
    }

    fn parse_array(&mut self) -> PResult<PseudoValue> {
        debug_assert_eq!(self.peek(), Some(b'['));
        self.i += 1; // consume '['
        let mut items = Vec::new();
        loop {
            self.skip_trivia();
            match self.peek() {
                None => return self.err("unterminated '[' array"),
                Some(b']') => {
                    self.i += 1;
                    break;
                }
                _ => {}
            }
            let v = self.parse_value()?;
            items.push(v);
            self.skip_ws();
            if self.peek() == Some(b',') {
                self.i += 1;
            }
        }
        Ok(PseudoValue::Array(items))
    }

    fn parse_number(&mut self) -> PResult<PseudoValue> {
        let start = self.i;
        if self.peek() == Some(b'-') {
            self.i += 1;
        }
        if self.starts_with("0x") || self.starts_with("0X") {
            self.i += 2;
            while matches!(self.peek(), Some(c) if c.is_ascii_hexdigit()) {
                self.i += 1;
            }
            return Ok(PseudoValue::Num(self.src[start..self.i].to_string()));
        }
        if self.starts_with("inf") {
            self.i += 3;
            return Ok(PseudoValue::Num(self.src[start..self.i].to_string()));
        }
        if self.starts_with("NaN") {
            self.i += 3;
            return Ok(PseudoValue::Num(self.src[start..self.i].to_string()));
        }
        while matches!(
            self.peek(),
            Some(c) if c.is_ascii_digit() || c == b'.' || c == b'e' || c == b'E'
                || c == b'+' || c == b'-'
        ) {
            self.i += 1;
        }
        let tok = self.src[start..self.i].to_string();
        if tok.is_empty() || tok == "-" {
            return self.err("malformed number");
        }
        Ok(PseudoValue::Num(tok))
    }

    fn read_ref_label(&mut self) -> String {
        let start = self.i;
        while let Some(c) = self.peek() {
            match c {
                b'\n' | b',' | b']' | b'}' => break,
                b'/' if self.s.get(self.i + 1) == Some(&b'/') => break,
                _ => self.i += 1,
            }
        }
        self.src[start..self.i].trim_end().to_string()
    }

    fn read_ident(&mut self) -> String {
        let start = self.i;
        while let Some(c) = self.peek() {
            if c == b'_' || c == b'-' || c.is_ascii_alphanumeric() {
                self.i += 1;
            } else {
                break;
            }
        }
        self.src[start..self.i].to_string()
    }

    fn read_bareword(&mut self) -> String {
        let start = self.i;
        while let Some(c) = self.peek() {
            if c == b'_' || c == b'-' || c == b':' || c.is_ascii_alphanumeric() {
                self.i += 1;
            } else {
                break;
            }
        }
        self.src[start..self.i].to_string()
    }

    fn parse_quoted_name(&mut self) -> PResult<String> {
        debug_assert_eq!(self.peek(), Some(b'\''));
        self.i += 1;
        let start = self.i;
        while let Some(c) = self.peek() {
            if c == b'\'' {
                let s = self.src[start..self.i].to_string();
                self.i += 1;
                return Ok(s);
            }
            self.i += 1;
        }
        self.err("unterminated 'name'")
    }

    fn parse_quoted_string(&mut self) -> PResult<String> {
        debug_assert_eq!(self.peek(), Some(b'"'));
        self.i += 1;
        let mut out = String::new();
        loop {
            let c = match self.bump() {
                None => return self.err("unterminated \"string\""),
                Some(c) => c,
            };
            match c {
                b'"' => return Ok(out),
                b'\\' => {
                    let e = self.bump().ok_or_else(|| ParseError {
                        offset: self.i,
                        msg: "dangling escape".into(),
                    })?;
                    match e {
                        b'"' => out.push('"'),
                        b'\\' => out.push('\\'),
                        b'n' => out.push('\n'),
                        b'r' => out.push('\r'),
                        b't' => out.push('\t'),
                        b'0' => out.push('\0'),
                        b'\'' => out.push('\''),
                        b'u' => {
                            // \u{XXXX}
                            if self.peek() != Some(b'{') {
                                return self.err("expected '{' after \\u");
                            }
                            self.i += 1;
                            let hstart = self.i;
                            while matches!(self.peek(), Some(h) if h.is_ascii_hexdigit()) {
                                self.i += 1;
                            }
                            let hex = &self.src[hstart..self.i];
                            if self.peek() != Some(b'}') {
                                return self.err("expected '}' after \\u{");
                            }
                            self.i += 1;
                            let cp = u32::from_str_radix(hex, 16)
                                .ok()
                                .and_then(char::from_u32)
                                .ok_or_else(|| ParseError {
                                    offset: hstart,
                                    msg: "invalid \\u code point".into(),
                                })?;
                            out.push(cp);
                        }
                        other => {
                            out.push(other as char);
                        }
                    }
                }
                _ => {
                    let bstart = self.i - 1;
                    while self.i < self.s.len() && (self.s[self.i] & 0xC0) == 0x80 {
                        self.i += 1;
                    }
                    out.push_str(&self.src[bstart..self.i]);
                }
            }
        }
    }
}
