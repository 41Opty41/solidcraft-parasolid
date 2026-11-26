//! Clean-room Parasolid XT binary (.x_b) and text (.x_t) serializers.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use crate::model::{
    Appearance, BrepModel, CurveData, CurveKind, NurbsCurve, NurbsSurface, Sense, SurfaceData,
    SurfaceKind, Transform3D,
};
use crate::schema::{builtin_schema, effective, FieldSpec, NodeSpec};
use crate::xb::{ParseResult, XtError, XtHeader, XtNode, XtValue, UNSET_I, UNSET_R};

const LINE_WIDTH: usize = 78;

fn build_common_header(format_kind: &str, schema_name: &str, ufs: i32) -> Vec<u8> {
    let sch = if schema_name.is_empty() {
        "sch_12000"
    } else {
        schema_name
    };
    let lines = [
        "**ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz**************************",
        "**PARASOLID !\"#$%&'()*+,-./:;<=>?@[\\]^_`{|}~0123456789**************************",
        "**PART1;",
        "MC=Unknown;",
        "MC_MODEL=unknown;",
        "MC_ID=unknown;",
        "OS=unknown;",
        "OS_RELEASE=unknown;",
        "FRU=xtopen;",
        "APPL=xtopen;",
        "SITE=unknown;",
        "USER=unknown;",
        &format!("FORMAT={format_kind};"),
        "GUISE=transmit;",
        "KEY=part_file;",
        "FILE=unknown;",
        "DATE=1-jan-2026;",
        "**PART2;",
        &format!("SCH={sch};"),
        &format!("USFLD_SIZE={ufs};"),
        "**PART3;",
        "**END_OF_HEADER*****************************************************************",
        "",
    ];
    lines.join("\n").into_bytes()
}

fn is_sch_4part(name: &str) -> bool {
    let mut parts = name.split('_');
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

fn enc_bin_index(buf: &mut Vec<u8>, val: Option<i64>) {
    match val {
        None | Some(0) => buf.extend_from_slice(&0i16.to_be_bytes()),
        Some(v) => {
            let wire = v + 1;
            if wire > 0 && wire <= 32767 {
                buf.extend_from_slice(&(wire as i16).to_be_bytes());
            } else {
                let mut q = wire / 32767;
                let mut r = wire % 32767;
                if r == 0 {
                    q -= 1;
                    r = 32767;
                }
                buf.extend_from_slice(&(-(r as i16)).to_be_bytes());
                buf.extend_from_slice(&(q as u16).to_be_bytes());
            }
        }
    }
}

fn enc_bin_scalar(buf: &mut Vec<u8>, code: char, val: &XtValue, form2: bool) -> Result<(), XtError> {
    match code {
        'u' | 'c' => {
            let b = match val {
                XtValue::Byte(v) => *v,
                XtValue::Int(v) => (*v & 0xFF) as u8,
                XtValue::Bool(v) => u8::from(*v),
                XtValue::Null => 0,
                _ => 0,
            };
            buf.push(b);
        }
        'l' => {
            let b = match val {
                XtValue::Bool(v) => u8::from(*v),
                XtValue::Int(v) => u8::from(*v != 0),
                XtValue::Byte(v) => u8::from(*v != 0),
                XtValue::Null => 0,
                _ => 0,
            };
            if form2 {
                buf.push(b);
            } else {
                buf.push(b);
            }
        }
        'n' => {
            let v = val.as_i64().map(|x| x as i16).unwrap_or(UNSET_I as i16);
            buf.extend_from_slice(&v.to_be_bytes());
        }
        'w' => {
            let v = val.as_i64().map(|x| x as u16).unwrap_or(0);
            buf.extend_from_slice(&v.to_be_bytes());
        }
        'd' | 't' => {
            let v = val.as_i64().map(|x| x as i32).unwrap_or(UNSET_I);
            buf.extend_from_slice(&v.to_be_bytes());
        }
        'p' => {
            let idx = match val {
                XtValue::Null => None,
                other => other.as_i64().filter(|&x| x != 0),
            };
            enc_bin_index(buf, idx);
        }
        'f' => {
            let f = val.as_f64().unwrap_or(UNSET_R);
            buf.extend_from_slice(&f.to_be_bytes());
        }
        'i' => {
            let [a, b] = match val {
                XtValue::Pair([a, b]) => [a.unwrap_or(UNSET_R), b.unwrap_or(UNSET_R)],
                _ => [UNSET_R, UNSET_R],
            };
            buf.extend_from_slice(&a.to_be_bytes());
            buf.extend_from_slice(&b.to_be_bytes());
        }
        'v' | 'h' => {
            let [x, y, z] = match val {
                XtValue::Vec3([x, y, z]) => {
                    [x.unwrap_or(UNSET_R), y.unwrap_or(UNSET_R), z.unwrap_or(UNSET_R)]
                }
                _ => [UNSET_R, UNSET_R, UNSET_R],
            };
            buf.extend_from_slice(&x.to_be_bytes());
            buf.extend_from_slice(&y.to_be_bytes());
            buf.extend_from_slice(&z.to_be_bytes());
        }
        'b' => {
            let arr = match val {
                XtValue::Box6(b) => *b,
                _ => [None; 6],
            };
            for item in arr {
                buf.extend_from_slice(&item.unwrap_or(UNSET_R).to_be_bytes());
            }
        }
        _ => {
            return Err(XtError::Unsupported(format!(
                "Unsupported XT binary field code for writer: {code:?}"
            )));
        }
    }
    Ok(())
}

fn fmt_text_float(val: Option<f64>) -> String {
    match val {
        None => "?".to_string(),
        Some(v) => {
            if !v.is_finite() {
                "?".to_string()
            } else if v == 0.0 {
                "0".to_string()
            } else {
                format!("{v}")
            }
        }
    }
}

fn enc_text_scalar(code: char, val: &XtValue) -> Result<String, XtError> {
    let s = match code {
        'u' | 'n' | 'w' | 'd' | 't' => match val {
            XtValue::Null => "?".to_string(),
            other => other
                .as_i64()
                .map(|i| i.to_string())
                .unwrap_or_else(|| "?".to_string()),
        },
        'l' => match val {
            XtValue::Null => "?".to_string(),
            other => {
                if other.as_bool().unwrap_or(false) {
                    "T".to_string()
                } else {
                    "F".to_string()
                }
            }
        },
        'p' => match val {
            XtValue::Null => "0".to_string(),
            other => other.as_i64().unwrap_or(0).to_string(),
        },
        'f' => fmt_text_float(val.as_f64()),
        'i' => match val {
            XtValue::Pair([a, b]) => format!("{} {}", fmt_text_float(*a), fmt_text_float(*b)),
            _ => "? ?".to_string(),
        },
        'v' | 'h' => match val {
            XtValue::Vec3([x, y, z]) => format!(
                "{} {} {}",
                fmt_text_float(*x),
                fmt_text_float(*y),
                fmt_text_float(*z)
            ),
            _ => "? ? ?".to_string(),
        },
        'b' => match val {
            XtValue::Box6(arr) => arr
                .iter()
                .map(|x| fmt_text_float(*x))
                .collect::<Vec<_>>()
                .join(" "),
            _ => "? ? ? ? ? ?".to_string(),
        },
        'c' => match val {
            XtValue::Null => "?".to_string(),
            XtValue::Byte(b) => (*b as char).to_string(),
            XtValue::Int(i) => ((*i & 0xFF) as u8 as char).to_string(),
            _ => "?".to_string(),
        },
        _ => {
            return Err(XtError::Unsupported(format!(
                "Unsupported XT text field code for writer: {code:?}"
            )));
        }
    };
    Ok(s)
}

fn infer_varlen(node: &XtNode, layout: &[FieldSpec]) -> usize {
    for f in layout {
        if f.elements == 1 {
            if let Some(vals) = node.field(&f.name) {
                return vals.len();
            }
        }
    }
    0
}

pub fn write_xb_nodes(nodes: &[XtNode], header: Option<&XtHeader>) -> Result<Vec<u8>, XtError> {
    let schema = builtin_schema();
    let hdr = header.cloned().unwrap_or_default();
    let form2 = hdr.form2;
    let schema_name = if !hdr.schema_name.is_empty() {
        if form2 || hdr.schema_name == "sch_12000" {
            hdr.schema_name.clone()
        } else {
            hdr.schema_name.clone()
        }
    } else if form2 {
        "SCH_12000_12000_12000".to_string()
    } else {
        "sch_12000".to_string()
    };
    let modeler = if hdr.modeler.is_empty() {
        " : TRANSMIT FILE created by xtopen".to_string()
    } else if hdr.modeler.starts_with(" : ") {
        hdr.modeler.clone()
    } else {
        format!(" : {}", hdr.modeler)
    };
    let ufs = hdr.user_field_size;
    let max_node = hdr.max_node.or_else(|| {
        if is_sch_4part(&schema_name) {
            Some(nodes.iter().map(|n| n.index).max().unwrap_or(0) as i32 + 1)
        } else {
            None
        }
    });

    let mut buf = build_common_header("binary", &schema_name, ufs);
    buf.extend_from_slice(b"PS\x00\x00");
    let mb = modeler.as_bytes();
    let sb = schema_name.as_bytes();
    buf.extend_from_slice(&(mb.len() as i16).to_be_bytes());
    buf.extend_from_slice(mb);
    buf.extend_from_slice(&(sb.len() as i32).to_be_bytes());
    buf.extend_from_slice(sb);
    if is_sch_4part(&schema_name) {
        buf.extend_from_slice(&(max_node.unwrap_or(0) as i16).to_be_bytes());
    }
    buf.extend_from_slice(&ufs.to_be_bytes());

    let mut seen: BTreeMap<i32, Vec<FieldSpec>> = BTreeMap::new();
    for n in nodes {
        buf.extend_from_slice(&(n.type_id as i16).to_be_bytes());
        let base_node = schema.get(&n.type_id).ok_or_else(|| {
            XtError::Unsupported(format!(
                "Cannot write unsupported XT node type {} ({})",
                n.name, n.type_id
            ))
        })?;
        if !seen.contains_key(&n.type_id) {
            buf.push(255);
            seen.insert(n.type_id, effective(base_node));
        }
        let layout = &seen[&n.type_id];
        if base_node.variable {
            let vlen = n
                .varlen
                .map(|v| v.max(0) as usize)
                .unwrap_or_else(|| infer_varlen(n, layout));
            buf.extend_from_slice(&(vlen as i32).to_be_bytes());
        }
        enc_bin_index(&mut buf, Some(n.index));

        for f in layout {
            if !f.transmitted {
                continue;
            }
            let vals = n.field(&f.name).unwrap_or(&[]);
            if !form2 && f.elements == 1 && !base_node.variable {
                buf.extend_from_slice(&(vals.len() as i32).to_be_bytes());
            }
            if f.code == 'c' {
                for v in vals {
                    enc_bin_scalar(&mut buf, 'c', v, form2)?;
                }
            } else {
                for v in vals {
                    enc_bin_scalar(&mut buf, f.code, v, form2)?;
                }
            }
        }
    }

    // Terminator record: type=1, index=0 (wire 1)
    buf.extend_from_slice(&1i16.to_be_bytes());
    buf.extend_from_slice(&1i16.to_be_bytes());
    Ok(buf)
}

pub fn write_xt_nodes(nodes: &[XtNode], header: Option<&XtHeader>) -> Result<Vec<u8>, XtError> {
    let schema = builtin_schema();
    let hdr = header.cloned().unwrap_or_default();
    let schema_name = if !hdr.schema_name.is_empty() {
        hdr.schema_name.clone()
    } else {
        "SCH_12000_12000_12000".to_string()
    };
    let modeler = if hdr.modeler.is_empty() {
        " : TRANSMIT FILE created by xtopen".to_string()
    } else if hdr.modeler.starts_with(" : ") {
        hdr.modeler.clone()
    } else {
        format!(" : {}", hdr.modeler)
    };
    let ufs = hdr.user_field_size;
    let form2 = hdr.form2 || schema_name.starts_with("SCH_");
    let max_node = hdr.max_node.or_else(|| {
        if is_sch_4part(&schema_name) {
            Some(nodes.iter().map(|n| n.index).max().unwrap_or(0) as i32 + 1)
        } else {
            None
        }
    });

    let mut out = build_common_header("text", &schema_name, ufs);
    let mut stream = String::with_capacity(nodes.len() * 64 + 128);
    let _ = write!(stream, "T{} {}", modeler.len(), modeler);
    let _ = write!(stream, "{} {}", schema_name.len(), schema_name);
    if is_sch_4part(&schema_name) {
        let _ = write!(stream, "{} ", max_node.unwrap_or(0));
    }
    let _ = write!(stream, "{} ", ufs);

    let mut seen: BTreeMap<i32, Vec<FieldSpec>> = BTreeMap::new();
    for n in nodes {
        let _ = write!(stream, "{} ", n.type_id);
        let base_node = schema.get(&n.type_id).ok_or_else(|| {
            XtError::Unsupported(format!(
                "Cannot write unsupported XT node type {} ({})",
                n.name, n.type_id
            ))
        })?;
        if !seen.contains_key(&n.type_id) {
            stream.push_str("255 ");
            seen.insert(n.type_id, effective(base_node));
        }
        let layout = &seen[&n.type_id];
        if base_node.variable {
            let vlen = n
                .varlen
                .map(|v| v.max(0) as usize)
                .unwrap_or_else(|| infer_varlen(n, layout));
            let _ = write!(stream, "{vlen} ");
        }
        let _ = write!(stream, "{} ", n.index);

        for f in layout {
            if !f.transmitted {
                continue;
            }
            let vals = n.field(&f.name).unwrap_or(&[]);
            if f.code == 'c' && f.elements == 1 {
                for v in vals {
                    let b = match v {
                        XtValue::Byte(b) => *b,
                        XtValue::Int(i) => (*i & 0xFF) as u8,
                        _ => b' ',
                    };
                    stream.push(b as char);
                }
                continue;
            }
            if !form2 && f.elements == 1 && !base_node.variable {
                let _ = write!(stream, "{} ", vals.len());
            }
            for v in vals {
                let tok = enc_text_scalar(f.code, v)?;
                if matches!(f.code, 'c' | 'l') && matches!(tok.as_str(), "T" | "F" | "+" | "-" | "?")
                {
                    stream.push_str(&tok);
                } else {
                    stream.push_str(&tok);
                    stream.push(' ');
                }
            }
        }
    }

    stream.push_str("1 0 ");
    // Line wrap safely on ASCII boundaries
    let bytes = stream.as_bytes();
    for chunk in bytes.chunks(LINE_WIDTH) {
        out.extend_from_slice(chunk);
        out.push(b'\n');
    }
    Ok(out)
}

pub fn xb_to_xt(data: &[u8]) -> Result<Vec<u8>, XtError> {
    let parsed = crate::parse_bytes(data)?;
    write_xt_nodes(&parsed.nodes, Some(&parsed.header))
}

pub fn xt_to_xb(data: &[u8]) -> Result<Vec<u8>, XtError> {
    let parsed = crate::parse_bytes(data)?;
    write_xb_nodes(&parsed.nodes, Some(&parsed.header))
}

struct NodeBuilder {
    schema: &'static BTreeMap<i32, NodeSpec>,
    nodes: Vec<XtNode>,
    next_idx: i64,
    attr_defs: BTreeMap<String, i64>,
}

impl NodeBuilder {
    fn new() -> Self {
        Self {
            schema: builtin_schema(),
            nodes: Vec::new(),
            next_idx: 1,
            attr_defs: BTreeMap::new(),
        }
    }

    fn alloc(&mut self) -> i64 {
        let i = self.next_idx;
        self.next_idx += 1;
        i
    }

    fn default_val(f: &FieldSpec, idx: i64) -> Vec<XtValue> {
        if f.name == "node_id" {
            return vec![XtValue::Int(idx)];
        }
        if f.elements == 1 {
            return Vec::new();
        }
        let count = f.elements.max(1);
        let one = match f.code {
            'p' => XtValue::Null,
            'u' | 'n' | 'w' | 'd' | 't' => XtValue::Int(0),
            'f' => XtValue::Float(0.0),
            'l' => XtValue::Bool(false),
            'c' => XtValue::Byte(b'+'),
            'i' => XtValue::Pair([Some(0.0), Some(0.0)]),
            'v' | 'h' => XtValue::Vec3([Some(0.0), Some(0.0), Some(0.0)]),
            'b' => XtValue::Box6([Some(0.0); 6]),
            _ => XtValue::Null,
        };
        vec![one; count]
    }

    fn add_node(
        &mut self,
        type_id: i32,
        idx_opt: Option<i64>,
        varlen: Option<i64>,
        overrides: Vec<(&str, Vec<XtValue>)>,
    ) -> i64 {
        let idx = idx_opt.unwrap_or_else(|| self.alloc());
        let spec = &self.schema[&type_id];
        let mut fields = Vec::new();
        for f in effective(spec) {
            if let Some((_, v)) = overrides.iter().find(|(k, _)| *k == f.name) {
                fields.push((f.name, v.clone()));
            } else {
                fields.push((f.name.clone(), Self::default_val(&f, idx)));
            }
        }
        self.nodes.push(XtNode {
            type_id,
            name: spec.name.clone(),
            index: idx,
            fields,
            varlen,
            offset: 0,
        });
        idx
    }

    fn set_field(&mut self, idx: i64, key: &str, val: Vec<XtValue>) {
        if let Some(n) = self.nodes.iter_mut().find(|n| n.index == idx) {
            for (k, v) in &mut n.fields {
                if k == key {
                    *v = val;
                    return;
                }
            }
        }
    }

    fn ensure_attr_def(&mut self, name: &str, field_code: i64) -> i64 {
        if let Some(&d) = self.attr_defs.get(name) {
            return d;
        }
        let bytes: Vec<XtValue> = name.bytes().map(XtValue::Byte).collect();
        let id_idx = self.add_node(
            79,
            None,
            Some(bytes.len() as i64),
            vec![("string", bytes)],
        );
        let def_idx = self.add_node(
            80,
            None,
            Some(1),
            vec![
                ("identifier", vec![XtValue::Int(id_idx)]),
                ("type_id", vec![XtValue::Int(1001 + self.attr_defs.len() as i64)]),
                ("legal_owners", vec![XtValue::Bool(true); 14]),
                ("fields", vec![XtValue::Byte(field_code as u8)]),
            ],
        );
        self.attr_defs.insert(name.to_string(), def_idx);
        def_idx
    }

    fn attach_appearance(&mut self, owner_idx: i64, app: &Appearance) {
        if let Some(ref nm) = app.name {
            if !nm.is_empty() {
                let def_idx = self.ensure_attr_def("SDL/TYSA_NAME", 1);
                let chars: Vec<XtValue> = nm.bytes().map(XtValue::Byte).collect();
                let val_idx = self.add_node(
                    84,
                    None,
                    Some(chars.len() as i64),
                    vec![("values", chars)],
                );
                self.add_node(
                    81,
                    None,
                    Some(1),
                    vec![
                        ("definition", vec![XtValue::Int(def_idx)]),
                        ("owner", vec![XtValue::Int(owner_idx)]),
                        ("fields", vec![XtValue::Int(val_idx)]),
                    ],
                );
            }
        }
        if let Some([r, g, b]) = app.color_rgb {
            let def_idx = self.ensure_attr_def("SDL/TYSA_COLOUR", 2);
            let vals = vec![XtValue::Float(r), XtValue::Float(g), XtValue::Float(b)];
            let val_idx = self.add_node(83, None, Some(3), vec![("values", vals)]);
            self.add_node(
                81,
                None,
                Some(1),
                vec![
                    ("definition", vec![XtValue::Int(def_idx)]),
                    ("owner", vec![XtValue::Int(owner_idx)]),
                    ("fields", vec![XtValue::Int(val_idx)]),
                ],
            );
        }
    }

    fn emit_nurbs_curve(&mut self, nc: &NurbsCurve) -> i64 {
        let mut raw_v = Vec::new();
        let vdim = if nc.is_pcurve {
            if nc.rational { 3 } else { 2 }
        } else if nc.rational {
            4
        } else {
            3
        };
        let ws = nc.weights.as_deref();
        if nc.is_pcurve {
            for (i, &[u, v]) in nc.poles_2d.iter().enumerate() {
                let w = if nc.rational {
                    ws.and_then(|arr| arr.get(i).copied()).unwrap_or(1.0)
                } else {
                    1.0
                };
                if nc.rational {
                    raw_v.extend([u * w, v * w, w].into_iter().map(XtValue::Float));
                } else {
                    raw_v.extend([u, v].into_iter().map(XtValue::Float));
                }
            }
        } else {
            for (i, &[x, y, z]) in nc.poles.iter().enumerate() {
                let w = if nc.rational {
                    ws.and_then(|arr| arr.get(i).copied()).unwrap_or(1.0)
                } else {
                    1.0
                };
                if nc.rational {
                    raw_v.extend([x * w, y * w, z * w, w].into_iter().map(XtValue::Float));
                } else {
                    raw_v.extend([x, y, z].into_iter().map(XtValue::Float));
                }
            }
        }
        let v_idx = self.add_node(45, None, Some(raw_v.len() as i64), vec![("vertices", raw_v)]);
        let k_vals: Vec<XtValue> = nc.knots.iter().copied().map(XtValue::Float).collect();
        let m_vals: Vec<XtValue> = nc
            .multiplicities
            .iter()
            .map(|&m| XtValue::Int(m as i64))
            .collect();
        let nk = k_vals.len() as i64;
        let k_idx = self.add_node(128, None, Some(nk), vec![("knots", k_vals)]);
        let m_idx = self.add_node(127, None, Some(nk), vec![("mult", m_vals)]);
        let n_verts = if nc.is_pcurve {
            nc.poles_2d.len()
        } else {
            nc.poles.len()
        } as i64;
        let nurbs_idx = self.add_node(
            136,
            None,
            None,
            vec![
                ("degree", vec![XtValue::Int(nc.degree as i64)]),
                ("n_vertices", vec![XtValue::Int(n_verts)]),
                ("vertex_dim", vec![XtValue::Int(vdim)]),
                ("n_knots", vec![XtValue::Int(nk)]),
                ("rational", vec![XtValue::Bool(nc.rational)]),
                ("periodic", vec![XtValue::Bool(nc.periodic)]),
                ("closed", vec![XtValue::Bool(nc.closed)]),
                ("bspline_vertices", vec![XtValue::Int(v_idx)]),
                ("knot_mult", vec![XtValue::Int(m_idx)]),
                ("knots", vec![XtValue::Int(k_idx)]),
            ],
        );
        self.add_node(
            134,
            None,
            None,
            vec![
                ("sense", vec![XtValue::Byte(b'+')]),
                ("nurbs", vec![XtValue::Int(nurbs_idx)]),
            ],
        )
    }

    fn emit_nurbs_surface(&mut self, ns: &NurbsSurface) -> i64 {
        let mut raw_v = Vec::new();
        let vdim = if ns.rational { 4 } else { 3 };
        let nu = ns.poles.len();
        let nv = ns.poles.first().map(Vec::len).unwrap_or(0);
        for (iu, row) in ns.poles.iter().enumerate() {
            for (iv, &[x, y, z]) in row.iter().enumerate() {
                let w = if ns.rational {
                    ns.weights
                        .as_ref()
                        .and_then(|wr| wr.get(iu))
                        .and_then(|r| r.get(iv))
                        .copied()
                        .unwrap_or(1.0)
                } else {
                    1.0
                };
                if ns.rational {
                    raw_v.extend([x * w, y * w, z * w, w].into_iter().map(XtValue::Float));
                } else {
                    raw_v.extend([x, y, z].into_iter().map(XtValue::Float));
                }
            }
        }
        let v_idx = self.add_node(45, None, Some(raw_v.len() as i64), vec![("vertices", raw_v)]);
        let uk_vals: Vec<XtValue> = ns.u_knots.iter().copied().map(XtValue::Float).collect();
        let um_vals: Vec<XtValue> = ns.u_mults.iter().map(|&m| XtValue::Int(m as i64)).collect();
        let vk_vals: Vec<XtValue> = ns.v_knots.iter().copied().map(XtValue::Float).collect();
        let vm_vals: Vec<XtValue> = ns.v_mults.iter().map(|&m| XtValue::Int(m as i64)).collect();
        let nuk = uk_vals.len() as i64;
        let nvk = vk_vals.len() as i64;
        let uk_idx = self.add_node(128, None, Some(nuk), vec![("knots", uk_vals)]);
        let um_idx = self.add_node(127, None, Some(nuk), vec![("mult", um_vals)]);
        let vk_idx = self.add_node(128, None, Some(nvk), vec![("knots", vk_vals)]);
        let vm_idx = self.add_node(127, None, Some(nvk), vec![("mult", vm_vals)]);
        let nurbs_idx = self.add_node(
            126,
            None,
            None,
            vec![
                ("u_periodic", vec![XtValue::Bool(ns.u_periodic)]),
                ("v_periodic", vec![XtValue::Bool(ns.v_periodic)]),
                ("u_degree", vec![XtValue::Int(ns.u_degree as i64)]),
                ("v_degree", vec![XtValue::Int(ns.v_degree as i64)]),
                ("n_u_vertices", vec![XtValue::Int(nu as i64)]),
                ("n_v_vertices", vec![XtValue::Int(nv as i64)]),
                ("n_u_knots", vec![XtValue::Int(nuk)]),
                ("n_v_knots", vec![XtValue::Int(nvk)]),
                ("rational", vec![XtValue::Bool(ns.rational)]),
                ("u_closed", vec![XtValue::Bool(ns.u_closed)]),
                ("v_closed", vec![XtValue::Bool(ns.v_closed)]),
                ("vertex_dim", vec![XtValue::Int(vdim)]),
                ("bspline_vertices", vec![XtValue::Int(v_idx)]),
                ("u_knot_mult", vec![XtValue::Int(um_idx)]),
                ("v_knot_mult", vec![XtValue::Int(vm_idx)]),
                ("u_knots", vec![XtValue::Int(uk_idx)]),
                ("v_knots", vec![XtValue::Int(vk_idx)]),
            ],
        );
        self.add_node(
            124,
            None,
            None,
            vec![
                ("sense", vec![XtValue::Byte(b'+')]),
                ("nurbs", vec![XtValue::Int(nurbs_idx)]),
            ],
        )
    }

    fn emit_curve(&mut self, c: &CurveData, owner: i64) -> i64 {
        let s = if c.sense.is_plus() { b'+' } else { b'-' };
        let v3 = |p: [f64; 3]| XtValue::Vec3([Some(p[0]), Some(p[1]), Some(p[2])]);
        match &c.kind {
            CurveKind::Line { origin, dir } => self.add_node(
                30,
                None,
                None,
                vec![
                    ("owner", vec![XtValue::Int(owner)]),
                    ("sense", vec![XtValue::Byte(s)]),
                    ("pvec", vec![v3(*origin)]),
                    ("direction", vec![v3(*dir)]),
                ],
            ),
            CurveKind::Circle {
                centre,
                normal,
                x_axis,
                radius,
            } => self.add_node(
                31,
                None,
                None,
                vec![
                    ("owner", vec![XtValue::Int(owner)]),
                    ("sense", vec![XtValue::Byte(s)]),
                    ("centre", vec![v3(*centre)]),
                    ("normal", vec![v3(*normal)]),
                    ("x_axis", vec![v3(*x_axis)]),
                    ("radius", vec![XtValue::Float(*radius)]),
                ],
            ),
            CurveKind::Ellipse {
                centre,
                normal,
                x_axis,
                major_radius,
                minor_radius,
            } => self.add_node(
                32,
                None,
                None,
                vec![
                    ("owner", vec![XtValue::Int(owner)]),
                    ("sense", vec![XtValue::Byte(s)]),
                    ("centre", vec![v3(*centre)]),
                    ("normal", vec![v3(*normal)]),
                    ("x_axis", vec![v3(*x_axis)]),
                    ("major_radius", vec![XtValue::Float(*major_radius)]),
                    ("minor_radius", vec![XtValue::Float(*minor_radius)]),
                ],
            ),
            CurveKind::Nurbs(nc) => self.emit_nurbs_curve(nc),
            _ => self.add_node(
                30,
                None,
                None,
                vec![
                    ("owner", vec![XtValue::Int(owner)]),
                    ("sense", vec![XtValue::Byte(s)]),
                    ("pvec", vec![v3([0.0, 0.0, 0.0])]),
                    ("direction", vec![v3([1.0, 0.0, 0.0])]),
                ],
            ),
        }
    }

    fn emit_surface(&mut self, sdata: &SurfaceData, owner: i64) -> i64 {
        let s = if sdata.sense.is_plus() { b'+' } else { b'-' };
        let v3 = |p: [f64; 3]| XtValue::Vec3([Some(p[0]), Some(p[1]), Some(p[2])]);
        match &sdata.kind {
            SurfaceKind::Plane {
                origin,
                normal,
                x_axis,
            } => self.add_node(
                50,
                None,
                None,
                vec![
                    ("owner", vec![XtValue::Int(owner)]),
                    ("sense", vec![XtValue::Byte(s)]),
                    ("pvec", vec![v3(*origin)]),
                    ("normal", vec![v3(*normal)]),
                    ("x_axis", vec![v3(*x_axis)]),
                ],
            ),
            SurfaceKind::Cylinder {
                origin,
                axis,
                x_axis,
                radius,
            } => self.add_node(
                51,
                None,
                None,
                vec![
                    ("owner", vec![XtValue::Int(owner)]),
                    ("sense", vec![XtValue::Byte(s)]),
                    ("pvec", vec![v3(*origin)]),
                    ("axis", vec![v3(*axis)]),
                    ("radius", vec![XtValue::Float(*radius)]),
                    ("x_axis", vec![v3(*x_axis)]),
                ],
            ),
            SurfaceKind::Cone {
                origin,
                axis,
                x_axis,
                radius,
                sin_half_angle,
                cos_half_angle,
            } => self.add_node(
                52,
                None,
                None,
                vec![
                    ("owner", vec![XtValue::Int(owner)]),
                    ("sense", vec![XtValue::Byte(s)]),
                    ("pvec", vec![v3(*origin)]),
                    ("axis", vec![v3(*axis)]),
                    ("radius", vec![XtValue::Float(*radius)]),
                    ("sin_half_angle", vec![XtValue::Float(*sin_half_angle)]),
                    ("cos_half_angle", vec![XtValue::Float(*cos_half_angle)]),
                    ("x_axis", vec![v3(*x_axis)]),
                ],
            ),
            SurfaceKind::Sphere {
                centre,
                axis,
                x_axis,
                radius,
            } => self.add_node(
                53,
                None,
                None,
                vec![
                    ("owner", vec![XtValue::Int(owner)]),
                    ("sense", vec![XtValue::Byte(s)]),
                    ("centre", vec![v3(*centre)]),
                    ("radius", vec![XtValue::Float(*radius)]),
                    ("axis", vec![v3(*axis)]),
                    ("x_axis", vec![v3(*x_axis)]),
                ],
            ),
            SurfaceKind::Torus {
                centre,
                axis,
                x_axis,
                major_radius,
                minor_radius,
            } => self.add_node(
                54,
                None,
                None,
                vec![
                    ("owner", vec![XtValue::Int(owner)]),
                    ("sense", vec![XtValue::Byte(s)]),
                    ("centre", vec![v3(*centre)]),
                    ("axis", vec![v3(*axis)]),
                    ("major_radius", vec![XtValue::Float(*major_radius)]),
                    ("minor_radius", vec![XtValue::Float(*minor_radius)]),
                    ("x_axis", vec![v3(*x_axis)]),
                ],
            ),
            SurfaceKind::Nurbs(ns) => self.emit_nurbs_surface(ns),
            SurfaceKind::Swept {
                direction,
                scale,
                profile: Some(prof),
                ..
            } => {
                let c_idx = self.emit_curve(prof, owner);
                self.add_node(
                    67,
                    None,
                    None,
                    vec![
                        ("owner", vec![XtValue::Int(owner)]),
                        ("sense", vec![XtValue::Byte(s)]),
                        ("section", vec![XtValue::Int(c_idx)]),
                        ("sweep", vec![v3(*direction)]),
                        ("scale", vec![XtValue::Float(*scale)]),
                    ],
                )
            }
            SurfaceKind::Spun {
                base,
                axis,
                x_axis,
                profile: Some(prof),
                ..
            } => {
                let c_idx = self.emit_curve(prof, owner);
                self.add_node(
                    68,
                    None,
                    None,
                    vec![
                        ("owner", vec![XtValue::Int(owner)]),
                        ("sense", vec![XtValue::Byte(s)]),
                        ("profile", vec![XtValue::Int(c_idx)]),
                        ("base", vec![v3(*base)]),
                        ("axis", vec![v3(*axis)]),
                        ("x_axis", vec![v3(*x_axis)]),
                        ("scale", vec![XtValue::Float(1.0)]),
                    ],
                )
            }
            SurfaceKind::Offset {
                offset,
                base_surface: Some(base),
                ..
            } => {
                let b_idx = self.emit_surface(base, owner);
                self.add_node(
                    60,
                    None,
                    None,
                    vec![
                        ("owner", vec![XtValue::Int(owner)]),
                        ("sense", vec![XtValue::Byte(s)]),
                        ("surface", vec![XtValue::Int(b_idx)]),
                        ("offset", vec![XtValue::Float(*offset)]),
                        ("scale", vec![XtValue::Float(1.0)]),
                    ],
                )
            }
            _ => self.add_node(
                50,
                None,
                None,
                vec![
                    ("owner", vec![XtValue::Int(owner)]),
                    ("sense", vec![XtValue::Byte(s)]),
                    ("pvec", vec![v3([0.0, 0.0, 0.0])]),
                    ("normal", vec![v3([0.0, 0.0, 1.0])]),
                    ("x_axis", vec![v3([1.0, 0.0, 0.0])]),
                ],
            ),
        }
    }
}

pub fn brep_to_xt_nodes(model: &BrepModel) -> ParseResult {
    let mut b = NodeBuilder::new();

    // Pre-allocate body, shell, region indices
    let mut body_xt: BTreeMap<i64, i64> = BTreeMap::new();
    let mut shell_xt: BTreeMap<i64, i64> = BTreeMap::new();
    let mut default_body_idx = None;
    let mut default_shell_idx = None;

    if model.bodies.is_empty() {
        let bi = b.alloc();
        let si = b.alloc();
        let ri = b.alloc();
        b.add_node(
            12,
            Some(bi),
            None,
            vec![
                ("state", vec![XtValue::Byte(1)]),
                ("body_type", vec![XtValue::Byte(1)]),
                ("nom_geom_state", vec![XtValue::Byte(1)]),
                ("shell", vec![XtValue::Int(si)]),
                ("region", vec![XtValue::Int(ri)]),
                ("res_size", vec![XtValue::Float(1000.0)]),
                ("res_linear", vec![XtValue::Float(1e-8)]),
            ],
        );
        b.add_node(
            13,
            Some(si),
            None,
            vec![("body", vec![XtValue::Int(bi)]), ("region", vec![XtValue::Int(ri)])],
        );
        b.add_node(
            19,
            Some(ri),
            None,
            vec![
                ("body", vec![XtValue::Int(bi)]),
                ("shell", vec![XtValue::Int(si)]),
                ("type", vec![XtValue::Byte(b'S')]),
            ],
        );
        default_body_idx = Some(bi);
        default_shell_idx = Some(si);
    } else {
        for body in &model.bodies {
            let bi = b.alloc();
            let ri = b.alloc();
            body_xt.insert(body.index, bi);
            let mut sh_ids = Vec::new();
            for &sh in &body.shell_indices {
                let si = b.alloc();
                shell_xt.insert(sh, si);
                sh_ids.push(si);
            }
            if sh_ids.is_empty() {
                sh_ids.push(b.alloc());
            }
            for (k, &si) in sh_ids.iter().enumerate() {
                let next_si = sh_ids.get(k + 1).copied();
                b.add_node(
                    13,
                    Some(si),
                    None,
                    vec![
                        ("body", vec![XtValue::Int(bi)]),
                        ("next", next_si.map(XtValue::Int).map(|x| vec![x]).unwrap_or(vec![XtValue::Null])),
                        ("region", vec![XtValue::Int(ri)]),
                    ],
                );
            }
            b.add_node(
                19,
                Some(ri),
                None,
                vec![
                    ("body", vec![XtValue::Int(bi)]),
                    ("shell", vec![XtValue::Int(sh_ids[0])]),
                    ("type", vec![XtValue::Byte(b'S')]),
                ],
            );
            b.add_node(
                12,
                Some(bi),
                None,
                vec![
                    ("state", vec![XtValue::Byte(1)]),
                    ("body_type", vec![XtValue::Byte(body.body_type.unwrap_or(1) as u8)]),
                    ("nom_geom_state", vec![XtValue::Byte(1)]),
                    ("shell", vec![XtValue::Int(sh_ids[0])]),
                    ("region", vec![XtValue::Int(ri)]),
                    ("res_size", vec![XtValue::Float(1000.0)]),
                    ("res_linear", vec![XtValue::Float(1e-8)]),
                ],
            );
            b.attach_appearance(bi, &body.appearance);
            if default_body_idx.is_none() {
                default_body_idx = Some(bi);
                default_shell_idx = Some(sh_ids[0]);
            }
        }
    }

    let root_body = default_body_idx.unwrap_or(1);
    let root_shell = default_shell_idx.unwrap_or(2);

    // Emit vertices
    let mut vert_xt: BTreeMap<i64, i64> = BTreeMap::new();
    for v in model.vertices.values() {
        let p_idx = b.add_node(
            29,
            None,
            None,
            vec![
                ("owner", vec![XtValue::Int(root_body)]),
                (
                    "pvec",
                    vec![XtValue::Vec3([
                        Some(v.point[0]),
                        Some(v.point[1]),
                        Some(v.point[2]),
                    ])],
                ),
            ],
        );
        let vi = b.add_node(
            18,
            None,
            None,
            vec![
                ("point", vec![XtValue::Int(p_idx)]),
                ("tolerance", vec![XtValue::Float(v.tolerance.max(1e-7))]),
                ("owner", vec![XtValue::Int(root_body)]),
            ],
        );
        vert_xt.insert(v.index, vi);
    }

    // Emit edges
    let mut edge_xt: BTreeMap<i64, i64> = BTreeMap::new();
    for e in model.edges.values() {
        let c_idx = b.emit_curve(&e.curve, root_body);
        let ei = b.add_node(
            16,
            None,
            None,
            vec![
                ("tolerance", vec![XtValue::Float(e.tolerance.max(1e-7))]),
                ("curve", vec![XtValue::Int(c_idx)]),
                ("owner", vec![XtValue::Int(root_body)]),
            ],
        );
        edge_xt.insert(e.index, ei);
    }

    // Emit faces, loops, halfedges
    let mut edge_fins: BTreeMap<i64, Vec<i64>> = BTreeMap::new();
    let mut faces_per_shell: BTreeMap<i64, Vec<i64>> = BTreeMap::new();

    for f in model.faces.values() {
        let sh_idx = f
            .shell_index
            .and_then(|si| shell_xt.get(&si).copied())
            .unwrap_or(root_shell);
        let fi = b.alloc();
        let surf_data = SurfaceData {
            sense: Sense::Plus,
            kind: f.surface.kind.clone(),
        };
        let s_idx = b.emit_surface(&surf_data, fi);

        let mut loop_ids = Vec::new();
        for lp in &f.loops {
            if lp.coedges.is_empty() {
                continue;
            }
            let li = b.alloc();
            let fin_ids: Vec<i64> = (0..lp.coedges.len()).map(|_| b.alloc()).collect();
            for (k, ce) in lp.coedges.iter().enumerate() {
                let fid = fin_ids[k];
                let fwd = fin_ids[(k + 1) % fin_ids.len()];
                let bwd = fin_ids[(k + fin_ids.len() - 1) % fin_ids.len()];
                let ei = edge_xt.get(&ce.edge_index).copied().unwrap_or(0);
                if ei != 0 {
                    edge_fins.entry(ei).or_default().push(fid);
                }
                let end_v = ce.end_vertex.and_then(|vi| vert_xt.get(&vi).copied());
                let s_ch = if ce.sense.is_plus() { b'+' } else { b'-' };
                b.add_node(
                    17,
                    Some(fid),
                    None,
                    vec![
                        ("loop", vec![XtValue::Int(li)]),
                        ("forward", vec![XtValue::Int(fwd)]),
                        ("backward", vec![XtValue::Int(bwd)]),
                        ("vertex", end_v.map(XtValue::Int).map(|x| vec![x]).unwrap_or(vec![XtValue::Null])),
                        ("edge", vec![XtValue::Int(ei)]),
                        ("sense", vec![XtValue::Byte(s_ch)]),
                    ],
                );
            }
            b.add_node(
                15,
                Some(li),
                None,
                vec![
                    ("halfedge", vec![XtValue::Int(fin_ids[0])]),
                    ("face", vec![XtValue::Int(fi)]),
                ],
            );
            loop_ids.push(li);
        }

        for k in 0..loop_ids.len() {
            if k + 1 < loop_ids.len() {
                b.set_field(loop_ids[k], "next", vec![XtValue::Int(loop_ids[k + 1])]);
            }
        }

        let first_loop = loop_ids.first().copied();
        let f_sense = if f.sense.is_plus() { b'+' } else { b'-' };
        b.add_node(
            14,
            Some(fi),
            None,
            vec![
                ("tolerance", vec![XtValue::Float(f.tolerance.max(1e-7))]),
                (
                    "loop",
                    first_loop
                        .map(XtValue::Int)
                        .map(|x| vec![x])
                        .unwrap_or(vec![XtValue::Null]),
                ),
                ("shell", vec![XtValue::Int(sh_idx)]),
                ("front_shell", vec![XtValue::Int(sh_idx)]),
                ("surface", vec![XtValue::Int(s_idx)]),
                ("sense", vec![XtValue::Byte(f_sense)]),
            ],
        );
        b.attach_appearance(fi, &f.appearance);
        faces_per_shell.entry(sh_idx).or_default().push(fi);
    }

    for (sh_idx, fids) in &faces_per_shell {
        if let Some(&first_f) = fids.first() {
            b.set_field(*sh_idx, "face", vec![XtValue::Int(first_f)]);
            b.set_field(*sh_idx, "front_face", vec![XtValue::Int(first_f)]);
        }
        for w in fids.windows(2) {
            b.set_field(w[0], "next", vec![XtValue::Int(w[1])]);
            b.set_field(w[1], "previous", vec![XtValue::Int(w[0])]);
        }
    }

    for (ei, fins) in &edge_fins {
        if let Some(&f0) = fins.first() {
            b.set_field(*ei, "halfedge", vec![XtValue::Int(f0)]);
        }
        if fins.len() == 2 {
            b.set_field(fins[0], "other", vec![XtValue::Int(fins[1])]);
            b.set_field(fins[1], "other", vec![XtValue::Int(fins[0])]);
        }
    }

    // Emit assemblies & instances if present
    if !model.assemblies.is_empty() && !model.instances.is_empty() {
        let mut asm_xt: BTreeMap<i64, i64> = BTreeMap::new();
        for a in &model.assemblies {
            asm_xt.insert(a.index, b.alloc());
        }
        let mut inst_xt: BTreeMap<i64, i64> = BTreeMap::new();
        for i in &model.instances {
            inst_xt.insert(i.index, b.alloc());
        }
        for a in &model.assemblies {
            let ai = asm_xt[&a.index];
            let sub_inst = a
                .instance_indices
                .first()
                .and_then(|ii| inst_xt.get(ii).copied());
            b.add_node(
                10,
                Some(ai),
                None,
                vec![
                    ("state", vec![XtValue::Byte(1)]),
                    ("type", vec![XtValue::Byte(1)]),
                    (
                        "sub_instance",
                        sub_inst
                            .map(XtValue::Int)
                            .map(|x| vec![x])
                            .unwrap_or(vec![XtValue::Null]),
                    ),
                ],
            );
            b.attach_appearance(ai, &a.appearance);
        }
        for i in &model.instances {
            let ii = inst_xt[&i.index];
            let part_xt = i
                .part_index
                .and_then(|pi| body_xt.get(&pi).or_else(|| asm_xt.get(&pi)).copied());
            let parent_xt = i.assembly_index.and_then(|ai| asm_xt.get(&ai).copied());
            let tf_idx = i.transform.as_ref().map(|t| {
                let mut rot_vals = Vec::with_capacity(9);
                for row in &t.rotation {
                    for &c in row {
                        rot_vals.push(XtValue::Float(c));
                    }
                }
                b.add_node(
                    100,
                    None,
                    None,
                    vec![
                        ("owner", vec![XtValue::Int(ii)]),
                        ("rotation_matrix", rot_vals),
                        (
                            "translation_vector",
                            vec![XtValue::Vec3([
                                Some(t.translation[0]),
                                Some(t.translation[1]),
                                Some(t.translation[2]),
                            ])],
                        ),
                        ("scale", vec![XtValue::Float(t.scale)]),
                        ("flag", vec![XtValue::Int(1)]),
                    ],
                )
            });
            b.add_node(
                11,
                Some(ii),
                None,
                vec![
                    ("type", vec![XtValue::Byte(i.inst_type.unwrap_or(1) as u8)]),
                    (
                        "part",
                        part_xt
                            .map(XtValue::Int)
                            .map(|x| vec![x])
                            .unwrap_or(vec![XtValue::Null]),
                    ),
                    (
                        "transform",
                        tf_idx
                            .map(XtValue::Int)
                            .map(|x| vec![x])
                            .unwrap_or(vec![XtValue::Null]),
                    ),
                    (
                        "assembly",
                        parent_xt
                            .map(XtValue::Int)
                            .map(|x| vec![x])
                            .unwrap_or(vec![XtValue::Null]),
                    ),
                ],
            );
            b.attach_appearance(ii, &i.appearance);
        }
        // Link sibling instances per assembly
        for a in &model.assemblies {
            let ids: Vec<i64> = a
                .instance_indices
                .iter()
                .filter_map(|ii| inst_xt.get(ii).copied())
                .collect();
            for w in ids.windows(2) {
                b.set_field(w[0], "next_in_part", vec![XtValue::Int(w[1])]);
                b.set_field(w[1], "prev_in_part", vec![XtValue::Int(w[0])]);
            }
        }
    }

    let highest = b.next_idx + 10;
    for &bi in body_xt.values().chain(default_body_idx.iter()) {
        b.set_field(bi, "highest_node_id", vec![XtValue::Int(highest)]);
    }

    b.nodes.sort_by_key(|n| n.index);
    ParseResult {
        header: XtHeader {
            format_kind: "binary".to_string(),
            modeler: " : TRANSMIT FILE created by xtopen".to_string(),
            schema_name: "sch_12000".to_string(),
            max_node: None,
            user_field_size: 0,
            form2: false,
            common_header: BTreeMap::new(),
        },
        nodes: b.nodes,
        remaining_bytes: 0,
    }
}

pub fn write_xb_from_brep(model: &BrepModel) -> Result<Vec<u8>, XtError> {
    let parsed = brep_to_xt_nodes(model);
    write_xb_nodes(&parsed.nodes, Some(&parsed.header))
}

pub fn write_xt_from_brep(model: &BrepModel) -> Result<Vec<u8>, XtError> {
    let mut parsed = brep_to_xt_nodes(model);
    parsed.header.format_kind = "text".to_string();
    parsed.header.schema_name = "SCH_12000_12000_12000".to_string();
    parsed.header.form2 = true;
    write_xt_nodes(&parsed.nodes, Some(&parsed.header))
}

#[allow(dead_code)]
fn _unused_tf(_t: &Transform3D) {}
