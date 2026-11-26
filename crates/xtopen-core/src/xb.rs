//! Clean-room binary Parasolid XT (.x_b, .x_p) parser.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use crate::schema::{builtin_schema, effective, FieldSpec};

pub const UNSET_I: i32 = -32764;
pub const UNSET_R: f64 = -3.14158e13;
const MAGIC_FILL: [u8; 8] = [0xc2, 0xbc, 0x92, 0x8f, 0x99, 0x6e, 0x00, 0x00];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum XtError {
    InvalidInput(String),
    Unsupported(String),
}

impl fmt::Display for XtError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            XtError::InvalidInput(msg) | XtError::Unsupported(msg) => write!(f, "{msg}"),
        }
    }
}

impl std::error::Error for XtError {}

#[derive(Clone, Debug, PartialEq)]
pub enum XtValue {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    Byte(u8),
    Pair([Option<f64>; 2]),
    Vec3([Option<f64>; 3]),
    Box6([Option<f64>; 6]),
}

impl XtValue {
    pub fn as_i64(&self) -> Option<i64> {
        match self {
            XtValue::Int(v) => Some(*v),
            XtValue::Byte(v) => Some(*v as i64),
            XtValue::Bool(v) => Some(if *v { 1 } else { 0 }),
            XtValue::Float(v) => Some(*v as i64),
            _ => None,
        }
    }

    pub fn as_f64(&self) -> Option<f64> {
        match self {
            XtValue::Float(v) => Some(*v),
            XtValue::Int(v) => Some(*v as f64),
            XtValue::Byte(v) => Some(*v as f64),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            XtValue::Bool(v) => Some(*v),
            XtValue::Int(v) => Some(*v != 0),
            XtValue::Byte(v) => Some(*v != 0),
            _ => None,
        }
    }

    pub fn as_vec3(&self) -> Option<[f64; 3]> {
        match self {
            XtValue::Vec3([Some(x), Some(y), Some(z)]) => Some([*x, *y, *z]),
            XtValue::Vec3([x, y, z]) => Some([x.unwrap_or(0.0), y.unwrap_or(0.0), z.unwrap_or(0.0)]),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct XtHeader {
    pub format_kind: String,
    pub modeler: String,
    pub schema_name: String,
    pub max_node: Option<i32>,
    pub user_field_size: i32,
    pub form2: bool,
    pub common_header: BTreeMap<String, String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct XtNode {
    pub type_id: i32,
    pub name: String,
    pub index: i64,
    pub fields: Vec<(String, Vec<XtValue>)>,
    pub varlen: Option<i64>,
    pub offset: usize,
}

impl XtNode {
    pub fn field(&self, key: &str) -> Option<&[XtValue]> {
        for (k, v) in &self.fields {
            if k == key {
                return Some(v.as_slice());
            }
        }
        None
    }

    pub fn get_first(&self, key: &str) -> Option<&XtValue> {
        let vals = self.field(key)?;
        vals.first()
    }

    pub fn get_i64(&self, key: &str) -> Option<i64> {
        self.get_first(key)?.as_i64()
    }

    pub fn get_f64(&self, key: &str) -> Option<f64> {
        self.get_first(key)?.as_f64()
    }

    pub fn get_bool(&self, key: &str) -> bool {
        self.get_first(key).and_then(XtValue::as_bool).unwrap_or(false)
    }

    pub fn get_byte(&self, key: &str) -> Option<u8> {
        match self.get_first(key)? {
            XtValue::Byte(b) => Some(*b),
            XtValue::Int(i) if (0..=255).contains(i) => Some(*i as u8),
            _ => None,
        }
    }

    pub fn get_vec3(&self, key: &str) -> Option<[f64; 3]> {
        self.get_first(key)?.as_vec3()
    }
}

#[derive(Clone, Debug, Default)]
pub struct ParseResult {
    pub header: XtHeader,
    pub nodes: Vec<XtNode>,
    pub remaining_bytes: usize,
}

struct Cur<'a> {
    d: &'a [u8],
    p: usize,
    le: bool,
    form2: bool,
}

impl<'a> Cur<'a> {
    fn new(data: &'a [u8], pos: usize, le: bool) -> Self {
        Self {
            d: data,
            p: pos,
            le,
            form2: false,
        }
    }

    fn take(&mut self, n: usize) -> Result<&'a [u8], XtError> {
        let end = self
            .p
            .checked_add(n)
            .ok_or_else(|| XtError::InvalidInput("Offset overflow while reading binary stream".into()))?;
        if end > self.d.len() {
            return Err(XtError::InvalidInput(format!(
                "Unexpected EOF at byte {} (wanted {n}, got {})",
                self.p,
                self.d.len().saturating_sub(self.p)
            )));
        }
        let slice = &self.d[self.p..end];
        self.p = end;
        Ok(slice)
    }

    fn peek(&self, n: usize) -> &[u8] {
        let end = self.p.saturating_add(n).min(self.d.len());
        &self.d[self.p..end]
    }

    fn u8(&mut self) -> Result<u8, XtError> {
        Ok(self.take(1)?[0])
    }

    fn i16_raw(&mut self) -> Result<i16, XtError> {
        let b: [u8; 2] = self.take(2)?.try_into().unwrap();
        Ok(if self.le {
            i16::from_le_bytes(b)
        } else {
            i16::from_be_bytes(b)
        })
    }

    fn i16(&mut self) -> Result<Option<i16>, XtError> {
        let v = self.i16_raw()?;
        Ok(if v as i32 == UNSET_I { None } else { Some(v) })
    }

    fn u16(&mut self) -> Result<u16, XtError> {
        let b: [u8; 2] = self.take(2)?.try_into().unwrap();
        Ok(if self.le {
            u16::from_le_bytes(b)
        } else {
            u16::from_be_bytes(b)
        })
    }

    fn i32(&mut self) -> Result<Option<i32>, XtError> {
        let b: [u8; 4] = self.take(4)?.try_into().unwrap();
        let v = if self.le {
            i32::from_le_bytes(b)
        } else {
            i32::from_be_bytes(b)
        };
        Ok(if v == UNSET_I { None } else { Some(v) })
    }

    fn f64(&mut self) -> Result<Option<f64>, XtError> {
        let b: [u8; 8] = self.take(8)?.try_into().unwrap();
        let v = if self.le {
            f64::from_le_bytes(b)
        } else {
            f64::from_be_bytes(b)
        };
        Ok(if v == UNSET_R { None } else { Some(v) })
    }

    fn index(&mut self) -> Result<Option<i64>, XtError> {
        let Some(rem) = self.i16()? else {
            return Ok(None);
        };
        if rem == 0 {
            return Ok(None);
        }
        if rem < 0 {
            let q = self.u16()? as i64;
            Ok(Some(q * 32767 + (-(rem as i64)) - 1))
        } else {
            Ok(Some((rem as i64) - 1))
        }
    }

    fn logical(&mut self) -> Result<bool, XtError> {
        let v = self.u8()?;
        match v {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(XtError::InvalidInput(format!(
                "Invalid logical byte {v} at offset {}",
                self.p
            ))),
        }
    }

    fn short_string(&mut self) -> Result<String, XtError> {
        let n = self.u8()? as usize;
        let bytes = self.take(n)?;
        Ok(String::from_utf8_lossy(bytes).into_owned())
    }
}

fn read_be_i16(bytes: &[u8]) -> Result<i16, XtError> {
    let arr: [u8; 2] = bytes
        .try_into()
        .map_err(|_| XtError::InvalidInput("Unexpected EOF reading i16".into()))?;
    Ok(i16::from_be_bytes(arr))
}

fn read_be_u16(bytes: &[u8]) -> Result<u16, XtError> {
    let arr: [u8; 2] = bytes
        .try_into()
        .map_err(|_| XtError::InvalidInput("Unexpected EOF reading u16".into()))?;
    Ok(u16::from_be_bytes(arr))
}

fn read_be_i32(bytes: &[u8]) -> Result<i32, XtError> {
    let arr: [u8; 4] = bytes
        .try_into()
        .map_err(|_| XtError::InvalidInput("Unexpected EOF reading i32".into()))?;
    Ok(i32::from_be_bytes(arr))
}

fn read_scalar(c: &mut Cur<'_>, code: char) -> Result<XtValue, XtError> {
    match code {
        'u' | 'c' => Ok(XtValue::Byte(c.u8()?)),
        'l' => {
            if c.form2 {
                Ok(XtValue::Byte(c.u8()?))
            } else {
                Ok(XtValue::Bool(c.logical()?))
            }
        }
        'n' => Ok(match c.i16()? {
            Some(v) => XtValue::Int(v as i64),
            None => XtValue::Null,
        }),
        'w' => Ok(XtValue::Int(c.u16()? as i64)),
        'd' | 't' => Ok(match c.i32()? {
            Some(v) => XtValue::Int(v as i64),
            None => XtValue::Null,
        }),
        'p' => Ok(match c.index()? {
            Some(v) => XtValue::Int(v),
            None => XtValue::Null,
        }),
        'f' => Ok(match c.f64()? {
            Some(v) => XtValue::Float(v),
            None => XtValue::Null,
        }),
        'i' => Ok(XtValue::Pair([c.f64()?, c.f64()?])),
        'v' | 'h' => Ok(XtValue::Vec3([c.f64()?, c.f64()?, c.f64()?])),
        'b' => Ok(XtValue::Box6([
            c.f64()?,
            c.f64()?,
            c.f64()?,
            c.f64()?,
            c.f64()?,
            c.f64()?,
        ])),
        _ => Err(XtError::InvalidInput(format!(
            "Unsupported XT binary field code: {code:?}"
        ))),
    }
}

pub(crate) fn parse_common_header(header_bytes: &[u8]) -> BTreeMap<String, String> {
    let text = String::from_utf8_lossy(header_bytes);
    let unwrapped = text.replace("\r\n", "\n").replace('\n', "");
    let mut meta = BTreeMap::new();
    for seg in unwrapped.split(';') {
        if let Some((raw_k, raw_v)) = seg.split_once('=') {
            let k = raw_k.trim().trim_start_matches('*');
            if k.starts_with("PART") {
                continue;
            }
            meta.insert(k.to_string(), raw_v.trim().to_string());
        }
    }
    meta
}

fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || needle.len() > haystack.len() {
        return None;
    }
    haystack.windows(needle.len()).position(|w| w == needle)
}

fn strip_common_header(data: &[u8]) -> Result<(&[u8], BTreeMap<String, String>), XtError> {
    if data.starts_with(b"PS\x00\x00") {
        return Ok((data, BTreeMap::new()));
    }
    let trimmed_start = data
        .iter()
        .position(|b| !matches!(b, b' ' | b'\t' | b'\r' | b'\n'))
        .unwrap_or(data.len());
    if !data[trimmed_start..].starts_with(b"**") {
        return Err(XtError::InvalidInput(
            "Missing Parasolid common header or PS\\x00\\x00 prefix".into(),
        ));
    }
    let marker = b"**END_OF_HEADER";
    let Some(i) = find_subslice(data, marker) else {
        return Err(XtError::InvalidInput(
            "Missing **END_OF_HEADER marker in XT header".into(),
        ));
    };
    let Some(rel_nl) = data[i..].iter().position(|&b| b == b'\n') else {
        return Err(XtError::InvalidInput(
            "Malformed **END_OF_HEADER line".into(),
        ));
    };
    let mut j = i + rel_nl + 1;
    while j < data.len() && (data[j] == b'\r' || data[j] == b'\n') {
        j += 1;
    }
    Ok((&data[j..], parse_common_header(&data[..i])))
}

fn read_embedded_field(c: &mut Cur<'_>) -> Result<FieldSpec, XtError> {
    let name = c.short_string()?;
    let ptr = read_be_i16(c.take(2)?)? as i32;
    let rem = read_be_i16(c.take(2)?)? as i32;
    let el = if rem < 0 {
        let q = read_be_u16(c.take(2)?)? as usize;
        q * 32767 + ((-rem) as usize)
    } else if rem == 0 {
        1
    } else {
        (rem - 1) as usize
    };

    let hi = *c
        .peek(1)
        .first()
        .ok_or_else(|| XtError::InvalidInput("Unexpected EOF reading embedded field".into()))?;
    let (code, transmitted) = if hi == 0 {
        c.take(1)?;
        let ch = c.take(1)?[0] as char;
        let mut raw = 2;
        while c.peek(1) == [0] && raw < 8 {
            c.take(1)?;
            raw += 1;
        }
        (ch, true)
    } else if ptr != 0 {
        ('p', true)
    } else {
        let tr = c.logical()?;
        let ch = c.take(1)?[0] as char;
        (ch, tr)
    };

    Ok(FieldSpec {
        name,
        code,
        ptr_class: ptr,
        elements: el,
        transmitted,
        rec: true,
    })
}

fn read_f2_field(c: &mut Cur<'_>) -> Result<FieldSpec, XtError> {
    let name = c.short_string()?;
    let ptr = read_be_i16(c.take(2)?)? as i32;
    let rem = read_be_i16(c.take(2)?)? as i32;
    let el = if rem < 0 {
        let q = read_be_u16(c.take(2)?)? as usize;
        q * 32767 + ((-rem) as usize)
    } else if rem == 0 || rem == 1 {
        1
    } else if rem == 2 {
        2
    } else {
        (rem - 1) as usize
    };
    let code = if ptr == 0 {
        let pk2 = c.take(2)?;
        pk2[1] as char
    } else {
        'p'
    };
    Ok(FieldSpec {
        name,
        code,
        ptr_class: ptr,
        elements: el,
        transmitted: true,
        rec: true,
    })
}

pub(crate) fn is_sch_4part(schema_name: &str) -> bool {
    let mut parts = schema_name.split('_');
    match (parts.next(), parts.next(), parts.next(), parts.next(), parts.next()) {
        (Some("SCH"), Some(a), Some(b), Some(c), None) => {
            !a.is_empty()
                && !b.is_empty()
                && !c.is_empty()
                && a.bytes().all(|x| x.is_ascii_digit())
                && b.bytes().all(|x| x.is_ascii_digit())
                && c.bytes().all(|x| x.is_ascii_digit())
        }
        _ => false,
    }
}

pub fn parse_xb(raw_data: &[u8]) -> Result<ParseResult, XtError> {
    let schema = builtin_schema();
    let (data, common_meta) = strip_common_header(raw_data)?;

    if find_subslice(&data[..data.len().min(96)], b"(deltas)").is_some() {
        return Err(XtError::Unsupported(
            "Delta transmit (.x_d) streams are not supported for solid import".into(),
        ));
    }

    if !data.starts_with(b"PS\x00\x00") {
        let prefix = String::from_utf8_lossy(&data[..data.len().min(8)]);
        return Err(XtError::InvalidInput(format!(
            "Expected PS\\x00\\x00 binary prefix, got {prefix:?}"
        )));
    }
    if data.len() < 10 {
        return Err(XtError::InvalidInput("Truncated Parasolid binary header".into()));
    }

    let mut c = Cur::new(data, 4, false);
    let mlen_i16 = c.i16_raw()?;
    if mlen_i16 < 0 {
        return Err(XtError::InvalidInput(format!(
            "Negative modeler string length {mlen_i16}"
        )));
    }
    let mlen = mlen_i16 as usize;
    let modeler = String::from_utf8_lossy(c.take(mlen)?).into_owned();

    let slen_i32 = read_be_i32(c.take(4)?)?;
    if slen_i32 < 0 {
        return Err(XtError::InvalidInput(format!(
            "Negative schema string length {slen_i32}"
        )));
    }
    let schema_name = String::from_utf8_lossy(c.take(slen_i32 as usize)?).into_owned();

    let mut max_node = None;
    if is_sch_4part(&schema_name) {
        max_node = Some(read_be_i16(c.take(2)?)? as i32);
    }
    let ufs = read_be_i32(c.take(4)?)?;

    let pk = c.p;
    if pk + 4 > data.len() {
        return Err(XtError::InvalidInput("Truncated stream before first node".into()));
    }
    let t1 = i16::from_be_bytes([data[pk], data[pk + 1]]);
    let op1 = data[pk + 3];
    let le1 = i16::from_le_bytes([data[pk], data[pk + 1]]);

    let form2 = (schema_name.starts_with("SCH_") && max_node.is_some())
        || (t1 > 1
            && t1 <= 4096
            && matches!(op1, b'C' | b'D' | b'I' | b'A' | b'Z')
            && (!schema.contains_key(&(t1 as i32)) || data[pk + 2] != 255)
            && !(le1 > 1 && le1 <= 4096 && data[pk + 2] == 255));
    c.form2 = form2;

    let mut layouts: BTreeMap<i32, Vec<FieldSpec>> = BTreeMap::new();
    let mut names: BTreeMap<i32, String> = BTreeMap::new();
    let mut variable_map: BTreeMap<i32, bool> = BTreeMap::new();
    let mut ext_types: BTreeSet<i32> = BTreeSet::new();
    let mut nodes: Vec<XtNode> = Vec::new();

    loop {
        let rec_start = c.p;
        if form2 && c.peek(8) == MAGIC_FILL {
            c.take(8)?;
            continue;
        }

        c.le = false;
        let Some(ntype_i16) = c.i16()? else {
            return Err(XtError::InvalidInput(format!(
                "Unset node type at offset {rec_start}"
            )));
        };
        let ntype = ntype_i16 as i32;
        if ntype < 0 {
            return Err(XtError::InvalidInput(format!(
                "Negative node type {ntype} at offset {rec_start}"
            )));
        }
        if ntype == 1 {
            if form2 {
                c.le = true;
            }
            let _ = c.index()?;
            break;
        }

        let base_node = schema.get(&ntype);
        if !layouts.contains_key(&ntype) {
            let flag = c.u8()? as usize;
            if let Some(bn) = base_node {
                if flag == 255 {
                    layouts.insert(ntype, effective(bn));
                    variable_map.insert(ntype, bn.variable);
                    names.insert(ntype, bn.name.clone());
                } else {
                    let base = effective(bn);
                    let next_b = c.peek(1).first().copied();
                    if form2
                        && flag != 1
                        && !matches!(next_b, Some(b'C' | b'D' | b'I' | b'A' | b'Z'))
                    {
                        let nm = c.short_string()?;
                        let _desc = c.short_string()?;
                        names.insert(ntype, nm);
                        let mut layout = Vec::with_capacity(flag);
                        for _ in 0..flag {
                            layout.push(read_f2_field(&mut c)?);
                        }
                        c.take(1)?;
                        layouts.insert(ntype, layout);
                        variable_map.insert(ntype, true);
                    } else if form2 && flag == 1 {
                        layouts.insert(ntype, base);
                        variable_map.insert(ntype, bn.variable);
                        names.insert(ntype, bn.name.clone());
                    } else {
                        let mut out = Vec::with_capacity(flag);
                        let mut pos = 0usize;
                        let mut append_phase = false;
                        loop {
                            let op = c.take(1)?[0];
                            match op {
                                b'C' => {
                                    if append_phase {
                                        return Err(XtError::InvalidInput(format!(
                                            "Edit op 'C' after 'A' at {}",
                                            c.p
                                        )));
                                    }
                                    if pos >= base.len() {
                                        return Err(XtError::InvalidInput(format!(
                                            "Edit op 'C' past base schema at {}",
                                            c.p
                                        )));
                                    }
                                    out.push(base[pos].clone());
                                    pos += 1;
                                }
                                b'D' => {
                                    if append_phase {
                                        return Err(XtError::InvalidInput(format!(
                                            "Edit op 'D' after 'A' at {}",
                                            c.p
                                        )));
                                    }
                                    pos += 1;
                                }
                                b'I' => {
                                    if append_phase {
                                        return Err(XtError::InvalidInput(format!(
                                            "Edit op 'I' after 'A' at {}",
                                            c.p
                                        )));
                                    }
                                    out.push(if form2 {
                                        read_f2_field(&mut c)?
                                    } else {
                                        read_embedded_field(&mut c)?
                                    });
                                }
                                b'A' => {
                                    if pos != base.len() {
                                        return Err(XtError::InvalidInput(format!(
                                            "Edit op 'A' before base end at {}",
                                            c.p
                                        )));
                                    }
                                    append_phase = true;
                                    out.push(if form2 {
                                        read_f2_field(&mut c)?
                                    } else {
                                        read_embedded_field(&mut c)?
                                    });
                                }
                                b'Z' => break,
                                other => {
                                    return Err(XtError::InvalidInput(format!(
                                        "Unknown schema edit op {} at {}",
                                        other as char, c.p
                                    )));
                                }
                            }
                        }
                        if out.len() != flag {
                            return Err(XtError::InvalidInput(format!(
                                "Schema edit produced {} fields, expected {flag}",
                                out.len()
                            )));
                        }
                        layouts.insert(ntype, out);
                        variable_map.insert(ntype, bn.variable);
                        names.insert(ntype, bn.name.clone());
                    }
                }
            } else {
                if flag == 255 {
                    return Err(XtError::InvalidInput(format!(
                        "Unknown node type {ntype} with flag 255 at {rec_start}"
                    )));
                }
                let nm = c.short_string()?;
                let _desc = c.short_string()?;
                let mut layout = Vec::with_capacity(flag);
                for _ in 0..flag {
                    layout.push(if form2 {
                        read_f2_field(&mut c)?
                    } else {
                        read_embedded_field(&mut c)?
                    });
                }
                if form2 {
                    layout.retain(|f| f.name != "node_id");
                    c.take(1)?;
                }
                layouts.insert(ntype, layout);
                names.insert(ntype, nm);
                variable_map.insert(ntype, form2);
            }
        }

        let layout = layouts
            .get(&ntype)
            .ok_or_else(|| XtError::InvalidInput(format!("Missing layout for type {ntype}")))?
            .clone();
        let node_name = names
            .get(&ntype)
            .cloned()
            .unwrap_or_else(|| format!("type_{ntype}"));
        let is_var = *variable_map.get(&ntype).unwrap_or(&false);

        if form2 {
            c.le = false;
        }

        let mut varlen: Option<i64> = None;
        let mut fidx_opt: Option<i64> = None;
        if form2 {
            if ext_types.contains(&ntype) {
                varlen = c.i32()?.map(|v| v as i64);
                fidx_opt = c.index()?;
                if fidx_opt.is_none() {
                    return Err(XtError::InvalidInput(format!(
                        "Zero node index at offset {rec_start}"
                    )));
                }
            } else if is_var {
                varlen = c.i32()?.map(|v| v as i64);
            }
        } else if is_var {
            varlen = c.i32()?.map(|v| v as i64);
        }

        let fidx = match fidx_opt {
            Some(v) => v,
            None => c.index()?.ok_or_else(|| {
                XtError::InvalidInput(format!("Zero node index at offset {rec_start}"))
            })?,
        };

        if form2 && base_node.is_none() {
            ext_types.insert(ntype);
        } else if form2 && names.contains_key(&ntype) && base_node.is_some_and(|b| b.name != node_name) {
            ext_types.insert(ntype);
        }

        let mut fields: Vec<(String, Vec<XtValue>)> = Vec::with_capacity(layout.len());
        for f in &layout {
            if !f.transmitted {
                continue;
            }
            let el = f.elements;
            let count: usize = if el == 0 {
                1
            } else if form2 && f.rec {
                if el == 2 {
                    varlen.unwrap_or(2).max(0) as usize
                } else {
                    el
                }
            } else if el == 1 {
                if f.name == "values" && ntype == 204 && !form2 {
                    let uv = fields
                        .iter()
                        .find(|(k, _)| k == "uv_type")
                        .and_then(|(_, v)| v.first())
                        .and_then(XtValue::as_i64)
                        .unwrap_or(1);
                    match uv {
                        0 => 0,
                        1 => 2,
                        2 => 3,
                        _ => 2,
                    }
                } else if let Some(vl) = varlen {
                    vl.max(0) as usize
                } else if form2 {
                    1
                } else {
                    c.i32()?.unwrap_or(0).max(0) as usize
                }
            } else if el == 2 && form2 {
                2
            } else if el == 2 {
                c.i32()?.unwrap_or(0).max(0) as usize
            } else {
                el
            };

            let vals = if f.code == 'c' {
                c.take(count)?
                    .iter()
                    .map(|&b| XtValue::Byte(b))
                    .collect::<Vec<_>>()
            } else {
                let mut v = Vec::with_capacity(count);
                for _ in 0..count {
                    v.push(read_scalar(&mut c, f.code)?);
                }
                v
            };
            fields.push((f.name.clone(), vals));
        }

        nodes.push(XtNode {
            type_id: ntype,
            name: node_name,
            index: fidx,
            fields,
            varlen,
            offset: rec_start,
        });
    }

    let format_kind = if common_meta.get("GUISE").map(String::as_str) == Some("transmit_partition") {
        "binary_partition"
    } else {
        "binary"
    };

    Ok(ParseResult {
        header: XtHeader {
            format_kind: format_kind.to_string(),
            modeler,
            schema_name,
            max_node,
            user_field_size: ufs,
            form2,
            common_header: common_meta,
        },
        nodes,
        remaining_bytes: data.len().saturating_sub(c.p),
    })
}
