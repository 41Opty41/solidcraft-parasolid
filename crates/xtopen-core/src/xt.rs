//! Clean-room text Parasolid XT (.x_t) parser.

use std::collections::{BTreeMap, BTreeSet};

use crate::schema::{builtin_schema, effective, FieldSpec};
use crate::xb::{is_sch_4part, parse_common_header, ParseResult, XtError, XtHeader, XtNode, XtValue};

struct TextCur<'a> {
    s: &'a [u8],
    p: usize,
}

impl<'a> TextCur<'a> {
    fn new(text: &'a [u8]) -> Self {
        Self { s: text, p: 0 }
    }

    fn skip_ws(&mut self) {
        while self.p < self.s.len() && matches!(self.s[self.p], b' ' | b'\t') {
            self.p += 1;
        }
    }

    fn peek_nonws(&self) -> Option<u8> {
        let mut i = self.p;
        while i < self.s.len() && matches!(self.s[i], b' ' | b'\t') {
            i += 1;
        }
        self.s.get(i).copied()
    }

    fn read_byte(&mut self) -> Result<u8, XtError> {
        if self.p >= self.s.len() {
            return Err(XtError::InvalidInput("Unexpected end of XT text stream".into()));
        }
        let b = self.s[self.p];
        self.p += 1;
        Ok(b)
    }

    fn read_nonws_byte(&mut self) -> Result<u8, XtError> {
        self.skip_ws();
        self.read_byte()
    }

    fn read_int(&mut self) -> Result<Option<i64>, XtError> {
        self.skip_ws();
        if self.p >= self.s.len() {
            return Err(XtError::InvalidInput("Unexpected EOF reading integer".into()));
        }
        if self.s[self.p] == b'?' {
            self.p += 1;
            return Ok(None);
        }
        let start = self.p;
        if matches!(self.s[self.p], b'+' | b'-') {
            self.p += 1;
        }
        let digits_start = self.p;
        while self.p < self.s.len() && self.s[self.p].is_ascii_digit() {
            self.p += 1;
        }
        if self.p == digits_start {
            let ctx = String::from_utf8_lossy(&self.s[start..self.s.len().min(start + 20)]);
            return Err(XtError::InvalidInput(format!(
                "Expected integer at offset {start}, got {ctx:?}"
            )));
        }
        let s = std::str::from_utf8(&self.s[start..self.p])
            .map_err(|_| XtError::InvalidInput("Non-UTF8 integer token".into()))?;
        let v = s
            .parse::<i64>()
            .map_err(|e| XtError::InvalidInput(format!("Invalid integer {s:?}: {e}")))?;
        Ok(Some(v))
    }

    fn read_float(&mut self) -> Result<Option<f64>, XtError> {
        self.skip_ws();
        if self.p >= self.s.len() {
            return Err(XtError::InvalidInput("Unexpected EOF reading float".into()));
        }
        if self.s[self.p] == b'?' {
            self.p += 1;
            return Ok(None);
        }
        let start = self.p;
        if matches!(self.s[self.p], b'+' | b'-') {
            self.p += 1;
        }
        let mut has_digits = false;
        while self.p < self.s.len() && self.s[self.p].is_ascii_digit() {
            has_digits = true;
            self.p += 1;
        }
        if self.p < self.s.len() && self.s[self.p] == b'.' {
            self.p += 1;
            while self.p < self.s.len() && self.s[self.p].is_ascii_digit() {
                has_digits = true;
                self.p += 1;
            }
        }
        if !has_digits {
            let ctx = String::from_utf8_lossy(&self.s[start..self.s.len().min(start + 20)]);
            return Err(XtError::InvalidInput(format!(
                "Expected float at offset {start}, got {ctx:?}"
            )));
        }
        if self.p < self.s.len() && matches!(self.s[self.p], b'e' | b'E') {
            let save = self.p;
            self.p += 1;
            if self.p < self.s.len() && matches!(self.s[self.p], b'+' | b'-') {
                self.p += 1;
            }
            let exp_start = self.p;
            while self.p < self.s.len() && self.s[self.p].is_ascii_digit() {
                self.p += 1;
            }
            if self.p == exp_start {
                self.p = save;
            }
        }
        let s = std::str::from_utf8(&self.s[start..self.p])
            .map_err(|_| XtError::InvalidInput("Non-UTF8 float token".into()))?;
        let v = s
            .parse::<f64>()
            .map_err(|e| XtError::InvalidInput(format!("Invalid float {s:?}: {e}")))?;
        Ok(Some(v))
    }

    fn read_logical(&mut self) -> Result<Option<i64>, XtError> {
        self.skip_ws();
        if self.p >= self.s.len() {
            return Err(XtError::InvalidInput("Unexpected EOF reading logical".into()));
        }
        match self.s[self.p] {
            b'T' => {
                self.p += 1;
                Ok(Some(1))
            }
            b'F' => {
                self.p += 1;
                Ok(Some(0))
            }
            b'?' => {
                self.p += 1;
                Ok(None)
            }
            b'0' | b'1' => {
                let v = self.read_int()?;
                Ok(v.map(|x| if x != 0 { 1 } else { 0 }))
            }
            ch => Err(XtError::InvalidInput(format!(
                "Expected logical at offset {}, got {:?}",
                self.p, ch as char
            ))),
        }
    }

    fn read_short_string(&mut self) -> Result<String, XtError> {
        let Some(len_i) = self.read_int()? else {
            return Ok(String::new());
        };
        if len_i < 0 {
            return Ok(String::new());
        }
        let len = len_i as usize;
        if self.p < self.s.len() && self.s[self.p] == b' ' {
            self.p += 1;
        }
        if self.p.saturating_add(len) > self.s.len() {
            return Err(XtError::InvalidInput(format!(
                "Unexpected EOF reading string of length {len}"
            )));
        }
        let out = String::from_utf8_lossy(&self.s[self.p..self.p + len]).into_owned();
        self.p += len;
        Ok(out)
    }
}

fn decode_xt_elements(rem: i64, cur: &mut TextCur<'_>) -> Result<usize, XtError> {
    if rem < 0 {
        let q = cur.read_int()?.unwrap_or(0).max(0) as usize;
        Ok(q * 32767 + ((-rem) as usize))
    } else if rem == 0 {
        Ok(0)
    } else {
        Ok(rem as usize)
    }
}

fn read_decl_field(cur: &mut TextCur<'_>, form2: bool) -> Result<FieldSpec, XtError> {
    let name = cur.read_short_string()?;
    let ptr = cur.read_int()?.unwrap_or(0) as i32;
    let rem = cur.read_int()?.unwrap_or(0);
    let el = decode_xt_elements(rem, cur)?;
    let (code, transmitted) = if ptr != 0 {
        ('p', true)
    } else {
        let tr = cur.read_int()?.unwrap_or(0) != 0;
        let ch = cur.read_nonws_byte()? as char;
        (ch, tr)
    };
    Ok(FieldSpec {
        name,
        code,
        ptr_class: ptr,
        elements: el,
        transmitted,
        rec: form2,
    })
}

fn read_text_scalar(cur: &mut TextCur<'_>, code: char) -> Result<XtValue, XtError> {
    match code {
        'u' | 'n' | 'w' | 'd' | 't' => Ok(match cur.read_int()? {
            Some(v) => XtValue::Int(v),
            None => XtValue::Null,
        }),
        'l' => Ok(match cur.read_logical()? {
            Some(v) => XtValue::Bool(v != 0),
            None => XtValue::Null,
        }),
        'p' => Ok(match cur.read_int()? {
            Some(idx) if idx != 0 => XtValue::Int(idx),
            _ => XtValue::Null,
        }),
        'f' => Ok(match cur.read_float()? {
            Some(v) => XtValue::Float(v),
            None => XtValue::Null,
        }),
        'i' => Ok(XtValue::Pair([cur.read_float()?, cur.read_float()?])),
        'v' | 'h' => Ok(XtValue::Vec3([
            cur.read_float()?,
            cur.read_float()?,
            cur.read_float()?,
        ])),
        'b' => Ok(XtValue::Box6([
            cur.read_float()?,
            cur.read_float()?,
            cur.read_float()?,
            cur.read_float()?,
            cur.read_float()?,
            cur.read_float()?,
        ])),
        'c' => {
            let ch = cur.read_nonws_byte()?;
            if ch == b'?' {
                Ok(XtValue::Null)
            } else {
                Ok(XtValue::Byte(ch))
            }
        }
        _ => Err(XtError::InvalidInput(format!(
            "Unsupported XT text field code: {code:?}"
        ))),
    }
}

fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || needle.len() > haystack.len() {
        return None;
    }
    haystack.windows(needle.len()).position(|w| w == needle)
}

pub fn parse_xt(raw: &[u8]) -> Result<ParseResult, XtError> {
    let base_schema = builtin_schema();

    let mut common_meta = BTreeMap::new();
    let mut body_bytes = raw;
    let trimmed_start = raw
        .iter()
        .position(|b| !matches!(b, b' ' | b'\t' | b'\r' | b'\n'))
        .unwrap_or(raw.len());
    if raw[trimmed_start..].starts_with(b"**") {
        let Some(i) = find_subslice(raw, b"**END_OF_HEADER") else {
            return Err(XtError::InvalidInput(
                "Missing **END_OF_HEADER marker in .x_t header".into(),
            ));
        };
        let Some(rel_nl) = raw[i..].iter().position(|&b| b == b'\n') else {
            return Err(XtError::InvalidInput(
                "Malformed **END_OF_HEADER line".into(),
            ));
        };
        common_meta = parse_common_header(&raw[..i]);
        body_bytes = &raw[i + rel_nl + 1..];
    }

    let mut stream = Vec::with_capacity(body_bytes.len());
    for &b in body_bytes {
        if b != b'\r' && b != b'\n' {
            stream.push(b);
        }
    }

    let mut cur = TextCur::new(&stream);
    cur.skip_ws();
    let tag = cur.read_byte()?;
    if tag != b'T' {
        return Err(XtError::InvalidInput(format!(
            "Expected 'T' preamble in .x_t stream, got {:?}",
            tag as char
        )));
    }

    let modeler = cur.read_short_string()?.trim().to_string();
    let schema_name = cur.read_short_string()?.trim().to_string();
    let max_node = if is_sch_4part(&schema_name) {
        cur.read_int()?.map(|v| v as i32)
    } else {
        None
    };
    let ufs = cur.read_int()?.unwrap_or(0) as i32;
    let form2 = schema_name.starts_with("SCH_");

    let mut layouts: BTreeMap<i32, Vec<FieldSpec>> = BTreeMap::new();
    let mut names: BTreeMap<i32, String> = BTreeMap::new();
    let mut variable_map: BTreeMap<i32, bool> = BTreeMap::new();
    let mut ext_types: BTreeSet<i32> = BTreeSet::new();
    let mut nodes: Vec<XtNode> = Vec::new();

    loop {
        cur.skip_ws();
        if cur.p >= cur.s.len() {
            break;
        }
        let rec_start = cur.p;
        let Some(ntype_i64) = cur.read_int()? else {
            return Err(XtError::InvalidInput(format!(
                "Unset node type at text offset {rec_start}"
            )));
        };
        let ntype = ntype_i64 as i32;
        if ntype == 1 {
            let _ = cur.read_int()?;
            break;
        }

        let base_node = base_schema.get(&ntype);
        if !layouts.contains_key(&ntype) {
            let Some(flag_i64) = cur.read_int()? else {
                return Err(XtError::InvalidInput(format!(
                    "Unset layout flag for type {ntype} at {rec_start}"
                )));
            };
            let flag = flag_i64.max(0) as usize;
            if let Some(bn) = base_node {
                if flag == 255 {
                    layouts.insert(ntype, effective(bn));
                    names.insert(ntype, bn.name.clone());
                    variable_map.insert(ntype, bn.variable);
                } else {
                    let base = effective(bn);
                    let nxt = cur.peek_nonws();
                    if matches!(nxt, Some(b'C' | b'D' | b'I' | b'A' | b'Z')) {
                        let mut out = Vec::new();
                        let mut pos = 0usize;
                        loop {
                            let op = cur.read_nonws_byte()?;
                            match op {
                                b'C' => {
                                    if pos >= base.len() {
                                        return Err(XtError::InvalidInput(format!(
                                            "Text schema 'C' past base schema at {}",
                                            cur.p
                                        )));
                                    }
                                    out.push(base[pos].clone());
                                    pos += 1;
                                }
                                b'D' => pos += 1,
                                b'I' | b'A' => out.push(read_decl_field(&mut cur, form2)?),
                                b'Z' => break,
                                other => {
                                    return Err(XtError::InvalidInput(format!(
                                        "Unknown text schema op {:?} at {}",
                                        other as char, cur.p
                                    )));
                                }
                            }
                        }
                        layouts.insert(ntype, out);
                        names.insert(ntype, bn.name.clone());
                        variable_map.insert(ntype, bn.variable);
                    } else if form2 && flag == 1 {
                        layouts.insert(ntype, base);
                        names.insert(ntype, bn.name.clone());
                        variable_map.insert(ntype, bn.variable);
                    } else {
                        let nm = cur.read_short_string()?;
                        let _desc = cur.read_short_string()?;
                        names.insert(ntype, nm);
                        let mut layout = Vec::with_capacity(flag);
                        for _ in 0..flag {
                            layout.push(read_decl_field(&mut cur, form2)?);
                        }
                        layouts.insert(ntype, layout);
                        ext_types.insert(ntype);
                        variable_map.insert(ntype, true);
                    }
                }
            } else {
                if flag == 255 {
                    return Err(XtError::InvalidInput(format!(
                        "Unknown node type {ntype} with flag 255 at {rec_start}"
                    )));
                }
                let nm = cur.read_short_string()?;
                let _desc = cur.read_short_string()?;
                let mut layout = Vec::with_capacity(flag);
                for _ in 0..flag {
                    layout.push(read_decl_field(&mut cur, form2)?);
                }
                if form2 {
                    layout.retain(|f| f.name != "node_id");
                }
                layouts.insert(ntype, layout);
                names.insert(ntype, nm);
                ext_types.insert(ntype);
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
        let is_var = ext_types.contains(&ntype) || *variable_map.get(&ntype).unwrap_or(&false);

        let varlen = if is_var { cur.read_int()? } else { None };
        let fidx = match cur.read_int()? {
            Some(idx) if idx != 0 => idx,
            other => {
                return Err(XtError::InvalidInput(format!(
                    "Invalid node index {other:?} at text offset {rec_start}"
                )));
            }
        };

        let mut fields = Vec::with_capacity(layout.len());
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
                if let Some(vl) = varlen {
                    vl.max(0) as usize
                } else if form2 {
                    1
                } else {
                    cur.read_int()?.unwrap_or(0).max(0) as usize
                }
            } else if el == 2 {
                if form2 {
                    2
                } else {
                    cur.read_int()?.unwrap_or(0).max(0) as usize
                }
            } else {
                el
            };

            let vals = if f.code == 'c' && el == 1 {
                if cur.p < cur.s.len() && cur.s[cur.p] == b' ' {
                    cur.p += 1;
                }
                if cur.p.saturating_add(count) > cur.s.len() {
                    return Err(XtError::InvalidInput(format!(
                        "Unexpected EOF reading {count} chars at {}",
                        cur.p
                    )));
                }
                let out = cur.s[cur.p..cur.p + count]
                    .iter()
                    .map(|&b| XtValue::Byte(b))
                    .collect();
                cur.p += count;
                out
            } else {
                let mut out = Vec::with_capacity(count);
                for _ in 0..count {
                    out.push(read_text_scalar(&mut cur, f.code)?);
                }
                out
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

    cur.skip_ws();
    Ok(ParseResult {
        header: XtHeader {
            format_kind: "text".to_string(),
            modeler,
            schema_name,
            max_node,
            user_field_size: ufs,
            form2,
            common_header: common_meta,
        },
        nodes,
        remaining_bytes: cur.s.len().saturating_sub(cur.p),
    })
}
