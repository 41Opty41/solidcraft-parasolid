//! High-level B-rep model extraction from parsed Parasolid XT nodes.

use std::collections::{BTreeMap, BTreeSet};

use crate::xb::{ParseResult, XtHeader, XtNode, XtValue};

#[derive(Clone, Debug, PartialEq)]
pub struct Transform3D {
    pub rotation: [[f64; 3]; 3],
    pub translation: [f64; 3],
    pub scale: f64,
}

impl Default for Transform3D {
    fn default() -> Self {
        Self::identity()
    }
}

impl Transform3D {
    pub fn identity() -> Self {
        Self {
            rotation: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
            translation: [0.0, 0.0, 0.0],
            scale: 1.0,
        }
    }

    pub fn from_xt(node: &XtNode) -> Self {
        let raw_rot = node_floats(Some(node), "rotation_matrix");
        let rotation = if raw_rot.len() >= 9 {
            [
                [
                    safe_float(Some(raw_rot[0]), 1.0),
                    safe_float(Some(raw_rot[1]), 0.0),
                    safe_float(Some(raw_rot[2]), 0.0),
                ],
                [
                    safe_float(Some(raw_rot[3]), 0.0),
                    safe_float(Some(raw_rot[4]), 1.0),
                    safe_float(Some(raw_rot[5]), 0.0),
                ],
                [
                    safe_float(Some(raw_rot[6]), 0.0),
                    safe_float(Some(raw_rot[7]), 0.0),
                    safe_float(Some(raw_rot[8]), 1.0),
                ],
            ]
        } else {
            [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]
        };
        let translation = safe_vec3(node.get_vec3("translation_vector"), [0.0, 0.0, 0.0]);
        let mut scale = safe_float(node.get_f64("scale"), 1.0);
        if scale.abs() < 1e-15 {
            scale = 1.0;
        }
        Self {
            rotation,
            translation,
            scale,
        }
    }

    pub fn det(&self) -> f64 {
        let r = &self.rotation;
        r[0][0] * (r[1][1] * r[2][2] - r[1][2] * r[2][1])
            - r[0][1] * (r[1][0] * r[2][2] - r[1][2] * r[2][0])
            + r[0][2] * (r[1][0] * r[2][1] - r[1][1] * r[2][0])
    }

    pub fn is_identity(&self) -> bool {
        let r = &self.rotation;
        let t = &self.translation;
        let rot_ok = (r[0][0] - 1.0).abs() < 1e-12
            && r[0][1].abs() < 1e-12
            && r[0][2].abs() < 1e-12
            && r[1][0].abs() < 1e-12
            && (r[1][1] - 1.0).abs() < 1e-12
            && r[1][2].abs() < 1e-12
            && r[2][0].abs() < 1e-12
            && r[2][1].abs() < 1e-12
            && (r[2][2] - 1.0).abs() < 1e-12;
        let tr_ok = t[0].abs() < 1e-12 && t[1].abs() < 1e-12 && t[2].abs() < 1e-12;
        rot_ok && tr_ok && (self.scale - 1.0).abs() < 1e-12
    }

    pub fn is_rigid(&self) -> bool {
        (self.scale - 1.0).abs() < 1e-9 && self.det() > 0.0
    }

    pub fn apply_point(&self, p: [f64; 3]) -> [f64; 3] {
        let r = &self.rotation;
        let s = self.scale;
        [
            s * (r[0][0] * p[0] + r[0][1] * p[1] + r[0][2] * p[2]) + self.translation[0],
            s * (r[1][0] * p[0] + r[1][1] * p[1] + r[1][2] * p[2]) + self.translation[1],
            s * (r[2][0] * p[0] + r[2][1] * p[1] + r[2][2] * p[2]) + self.translation[2],
        ]
    }

    pub fn apply_dir(&self, d: [f64; 3]) -> [f64; 3] {
        let r = &self.rotation;
        [
            r[0][0] * d[0] + r[0][1] * d[1] + r[0][2] * d[2],
            r[1][0] * d[0] + r[1][1] * d[1] + r[1][2] * d[2],
            r[2][0] * d[0] + r[2][1] * d[1] + r[2][2] * d[2],
        ]
    }

    pub fn compose(&self, child: &Transform3D) -> Transform3D {
        let mut rot = [[0.0; 3]; 3];
        for i in 0..3 {
            for j in 0..3 {
                rot[i][j] = self.rotation[i][0] * child.rotation[0][j]
                    + self.rotation[i][1] * child.rotation[1][j]
                    + self.rotation[i][2] * child.rotation[2][j];
            }
        }
        let translation = self.apply_point(child.translation);
        Transform3D {
            rotation: rot,
            translation,
            scale: self.scale * child.scale,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum AttributeValue {
    Text(String),
    Reals(Vec<f64>),
    Ints(Vec<i64>),
    Vec3s(Vec<[f64; 3]>),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Attribute {
    pub index: i64,
    pub name: String,
    pub owner_index: Option<i64>,
    pub values: Vec<AttributeValue>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Appearance {
    pub color_rgb: Option<[f64; 3]>,
    pub transparency: Option<f64>,
    pub name: Option<String>,
    pub layer: Option<i64>,
    pub density: Option<f64>,
}

impl Appearance {
    pub fn apply_attribute(&mut self, attr: &Attribute) {
        let upper = attr.name.to_ascii_uppercase();
        if upper.ends_with("TYSA_COLOUR") || upper.ends_with("TYSA_COLOR") {
            let mut reals = Vec::new();
            for v in &attr.values {
                if let AttributeValue::Reals(rs) = v {
                    reals.extend_from_slice(rs);
                }
            }
            if reals.len() >= 3 {
                self.color_rgb = Some([
                    reals[0].clamp(0.0, 1.0),
                    reals[1].clamp(0.0, 1.0),
                    reals[2].clamp(0.0, 1.0),
                ]);
            }
        } else if upper.ends_with("TYSA_NAME") {
            for v in &attr.values {
                if let AttributeValue::Text(s) = v {
                    let trimmed = s.trim();
                    if !trimmed.is_empty() {
                        self.name = Some(trimmed.to_string());
                        break;
                    }
                }
            }
        } else if upper.ends_with("TYSA_TRANSPARENCY") || upper.ends_with("TYSA_TRANSLUCENCY") {
            for v in &attr.values {
                if let AttributeValue::Reals(rs) = v {
                    if let Some(&t) = rs.first() {
                        if t.is_finite() {
                            self.transparency = Some(t.clamp(0.0, 1.0));
                            break;
                        }
                    }
                }
            }
        } else if upper.ends_with("TYSA_DENSITY") {
            for v in &attr.values {
                if let AttributeValue::Reals(rs) = v {
                    if let Some(&d) = rs.first() {
                        if d.is_finite() && d > 0.0 {
                            self.density = Some(d);
                            break;
                        }
                    }
                }
            }
        } else if upper.ends_with("TYSA_LAYER") {
            for v in &attr.values {
                if let AttributeValue::Ints(is) = v {
                    if let Some(&l) = is.first() {
                        self.layer = Some(l);
                        break;
                    }
                }
            }
        }
    }

    pub fn inherit_from(&self, parent: &Appearance) -> Appearance {
        Appearance {
            color_rgb: self.color_rgb.or(parent.color_rgb),
            transparency: self.transparency.or(parent.transparency),
            name: self.name.clone().or_else(|| parent.name.clone()),
            layer: self.layer.or(parent.layer),
            density: self.density.or(parent.density),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ConversionWarning {
    pub code: String,
    pub subject: String,
    pub message: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sense {
    Plus,
    Minus,
}

impl Sense {
    pub fn is_plus(self) -> bool {
        matches!(self, Sense::Plus)
    }

    pub fn from_xt(val: Option<&XtValue>) -> Self {
        match val {
            Some(XtValue::Byte(b)) => {
                if *b == 0 || *b == b'-' {
                    Sense::Minus
                } else {
                    Sense::Plus
                }
            }
            Some(XtValue::Int(i)) => {
                if *i == 0 || *i == b'-' as i64 {
                    Sense::Minus
                } else {
                    Sense::Plus
                }
            }
            _ => Sense::Plus,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct NurbsCurve {
    pub degree: usize,
    pub poles: Vec<[f64; 3]>,
    pub knots: Vec<f64>,
    pub multiplicities: Vec<usize>,
    pub weights: Option<Vec<f64>>,
    pub periodic: bool,
    pub closed: bool,
    pub rational: bool,
    pub vertex_dim: usize,
    pub is_pcurve: bool,
    pub poles_2d: Vec<[f64; 2]>,
    pub host_surface_index: Option<i64>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct NurbsSurface {
    pub u_degree: usize,
    pub v_degree: usize,
    pub u_knots: Vec<f64>,
    pub u_mults: Vec<usize>,
    pub v_knots: Vec<f64>,
    pub v_mults: Vec<usize>,
    pub poles: Vec<Vec<[f64; 3]>>,
    pub weights: Option<Vec<Vec<f64>>>,
    pub u_periodic: bool,
    pub v_periodic: bool,
    pub u_closed: bool,
    pub v_closed: bool,
    pub rational: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub enum SurfaceKind {
    Plane {
        origin: [f64; 3],
        normal: [f64; 3],
        x_axis: [f64; 3],
    },
    Cylinder {
        origin: [f64; 3],
        axis: [f64; 3],
        x_axis: [f64; 3],
        radius: f64,
    },
    Cone {
        origin: [f64; 3],
        axis: [f64; 3],
        x_axis: [f64; 3],
        radius: f64,
        sin_half_angle: f64,
        cos_half_angle: f64,
    },
    Sphere {
        centre: [f64; 3],
        axis: [f64; 3],
        x_axis: [f64; 3],
        radius: f64,
    },
    Torus {
        centre: [f64; 3],
        axis: [f64; 3],
        x_axis: [f64; 3],
        major_radius: f64,
        minor_radius: f64,
    },
    Nurbs(Box<NurbsSurface>),
    Swept {
        direction: [f64; 3],
        scale: f64,
        profile_index: Option<i64>,
        profile: Option<Box<CurveData>>,
    },
    Spun {
        base: [f64; 3],
        axis: [f64; 3],
        x_axis: [f64; 3],
        profile_index: Option<i64>,
        profile: Option<Box<CurveData>>,
    },
    Offset {
        offset: f64,
        base_surface_index: Option<i64>,
        base_surface: Option<Box<SurfaceData>>,
    },
    Unsupported(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct SurfaceData {
    pub sense: Sense,
    pub kind: SurfaceKind,
}

impl SurfaceData {
    pub fn type_name(&self) -> &str {
        match &self.kind {
            SurfaceKind::Plane { .. } => "plane",
            SurfaceKind::Cylinder { .. } => "cylinder",
            SurfaceKind::Cone { .. } => "cone",
            SurfaceKind::Sphere { .. } => "sphere",
            SurfaceKind::Torus { .. } => "torus",
            SurfaceKind::Nurbs(_) => "nurbs",
            SurfaceKind::Swept { .. } => "swept",
            SurfaceKind::Spun { .. } => "spun",
            SurfaceKind::Offset { .. } => "offset",
            SurfaceKind::Unsupported(s) => s.as_str(),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum CurveKind {
    Line {
        origin: [f64; 3],
        dir: [f64; 3],
    },
    Circle {
        centre: [f64; 3],
        normal: [f64; 3],
        x_axis: [f64; 3],
        radius: f64,
    },
    Ellipse {
        centre: [f64; 3],
        normal: [f64; 3],
        x_axis: [f64; 3],
        major_radius: f64,
        minor_radius: f64,
    },
    Nurbs(Box<NurbsCurve>),
    Intersection {
        points: Vec<[f64; 3]>,
    },
    Unsupported(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct CurveData {
    pub sense: Sense,
    pub kind: CurveKind,
    pub sp_surface: Option<SurfaceData>,
}

impl CurveData {
    pub fn type_name(&self) -> &str {
        match &self.kind {
            CurveKind::Line { .. } => "line",
            CurveKind::Circle { .. } => "circle",
            CurveKind::Ellipse { .. } => "ellipse",
            CurveKind::Nurbs(_) => "nurbs",
            CurveKind::Intersection { .. } => "intersection",
            CurveKind::Unsupported(s) => s.as_str(),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Vertex {
    pub index: i64,
    pub node_id: i64,
    pub point: [f64; 3],
    pub tolerance: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Edge {
    pub index: i64,
    pub node_id: i64,
    pub sense: Sense,
    pub tolerance: f64,
    pub curve_index: Option<i64>,
    pub curve: CurveData,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Coedge {
    pub edge_index: i64,
    pub sense: Sense,
    pub start_vertex: Option<i64>,
    pub end_vertex: Option<i64>,
    pub fin_index: i64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Loop {
    pub index: i64,
    pub coedges: Vec<Coedge>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Face {
    pub index: i64,
    pub node_id: i64,
    pub shell_index: Option<i64>,
    pub sense: Sense,
    pub face_sense: Sense,
    pub surface_sense: Sense,
    pub surface_index: Option<i64>,
    pub surface: SurfaceData,
    pub loops: Vec<Loop>,
    pub tolerance: f64,
    pub appearance: Appearance,
}

#[derive(Clone, Debug, PartialEq)]
pub struct BodyInfo {
    pub index: i64,
    pub node_id: i64,
    pub body_type: Option<i64>,
    pub shell_indices: Vec<i64>,
    pub face_count: usize,
    pub name: Option<String>,
    pub appearance: Appearance,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AssemblyInfo {
    pub index: i64,
    pub node_id: i64,
    pub name: Option<String>,
    pub appearance: Appearance,
    pub instance_indices: Vec<i64>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct InstanceInfo {
    pub index: i64,
    pub node_id: i64,
    pub inst_type: Option<i64>,
    pub assembly_index: Option<i64>,
    pub part_index: Option<i64>,
    pub transform: Option<Transform3D>,
    pub name: Option<String>,
    pub appearance: Appearance,
}

#[derive(Clone, Debug)]
pub struct BrepModel {
    pub header: XtHeader,
    pub bodies: Vec<BodyInfo>,
    pub vertices: BTreeMap<i64, Vertex>,
    pub edges: BTreeMap<i64, Edge>,
    pub faces: BTreeMap<i64, Face>,
    pub surfaces: BTreeMap<i64, SurfaceData>,
    pub curves: BTreeMap<i64, CurveData>,
    pub assemblies: Vec<AssemblyInfo>,
    pub instances: Vec<InstanceInfo>,
    pub attributes: Vec<Attribute>,
    pub warnings: Vec<ConversionWarning>,
}

fn safe_float(val: Option<f64>, default: f64) -> f64 {
    match val {
        Some(v) if v.is_finite() => v,
        _ => default,
    }
}

fn safe_vec3(raw: Option<[f64; 3]>, default: [f64; 3]) -> [f64; 3] {
    match raw {
        Some([x, y, z]) => [
            if x.is_finite() { x } else { default[0] },
            if y.is_finite() { y } else { default[1] },
            if z.is_finite() { z } else { default[2] },
        ],
        None => default,
    }
}

fn prune_knots(knots: &[f64], mults: &[i64]) -> (Vec<f64>, Vec<usize>) {
    if mults.is_empty() || mults.len() != knots.len() {
        return (
            knots.to_vec(),
            mults.iter().map(|&m| m.max(0) as usize).collect(),
        );
    }
    let mut out_k = Vec::with_capacity(knots.len());
    let mut out_m = Vec::with_capacity(mults.len());
    for (&k, &m) in knots.iter().zip(mults.iter()) {
        if m > 0 {
            out_k.push(k);
            out_m.push(m as usize);
        }
    }
    (out_k, out_m)
}

fn node_floats(node: Option<&XtNode>, field: &str) -> Vec<f64> {
    node.and_then(|n| n.field(field))
        .map(|vals| vals.iter().filter_map(XtValue::as_f64).collect())
        .unwrap_or_default()
}

fn node_ints(node: Option<&XtNode>, field: &str) -> Vec<i64> {
    node.and_then(|n| n.field(field))
        .map(|vals| vals.iter().filter_map(XtValue::as_i64).collect())
        .unwrap_or_default()
}

fn extract_nurbs_surface(n: &XtNode, nodes: &BTreeMap<i64, &XtNode>) -> NurbsSurface {
    let nu = n.get_i64("n_u_vertices").unwrap_or(0).max(0) as usize;
    let nv = n.get_i64("n_v_vertices").unwrap_or(0).max(0) as usize;
    let vdim = n.get_i64("vertex_dim").unwrap_or(3).max(1) as usize;
    let rat = n.get_bool("rational");

    let v_node = n.get_i64("bspline_vertices").and_then(|i| nodes.get(&i).copied());
    let raw_v = node_floats(v_node, "vertices");

    let uk_raw = node_floats(n.get_i64("u_knots").and_then(|i| nodes.get(&i).copied()), "knots");
    let vk_raw = node_floats(n.get_i64("v_knots").and_then(|i| nodes.get(&i).copied()), "knots");
    let um_raw = node_ints(n.get_i64("u_knot_mult").and_then(|i| nodes.get(&i).copied()), "mult");
    let vm_raw = node_ints(n.get_i64("v_knot_mult").and_then(|i| nodes.get(&i).copied()), "mult");

    let (u_knots, u_mults) = prune_knots(&uk_raw, &um_raw);
    let (v_knots, v_mults) = prune_knots(&vk_raw, &vm_raw);

    let mut poles = Vec::with_capacity(nu);
    let mut weights = if rat { Some(Vec::with_capacity(nu)) } else { None };
    for iu in 0..nu {
        let mut row_p = Vec::with_capacity(nv);
        let mut row_w = Vec::with_capacity(nv);
        for iv in 0..nv {
            let idx = (iu * nv + iv) * vdim;
            if idx + 2 >= raw_v.len() {
                row_p.push([0.0, 0.0, 0.0]);
                if rat {
                    row_w.push(1.0);
                }
                continue;
            }
            if rat && vdim >= 4 && idx + 3 < raw_v.len() {
                let w = if raw_v[idx + 3] != 0.0 { raw_v[idx + 3] } else { 1.0 };
                row_p.push([raw_v[idx] / w, raw_v[idx + 1] / w, raw_v[idx + 2] / w]);
                row_w.push(w);
            } else {
                row_p.push([raw_v[idx], raw_v[idx + 1], raw_v[idx + 2]]);
                if rat {
                    row_w.push(1.0);
                }
            }
        }
        poles.push(row_p);
        if let Some(ref mut ws) = weights {
            ws.push(row_w);
        }
    }

    NurbsSurface {
        u_degree: n.get_i64("u_degree").unwrap_or(1).max(1) as usize,
        v_degree: n.get_i64("v_degree").unwrap_or(1).max(1) as usize,
        u_knots,
        u_mults,
        v_knots,
        v_mults,
        poles,
        weights,
        u_periodic: n.get_bool("u_periodic"),
        v_periodic: n.get_bool("v_periodic"),
        u_closed: n.get_bool("u_closed"),
        v_closed: n.get_bool("v_closed"),
        rational: rat,
    }
}

fn extract_nurbs_curve(
    n: &XtNode,
    nodes: &BTreeMap<i64, &XtNode>,
    host_surface_index: Option<i64>,
) -> NurbsCurve {
    let nc = n.get_i64("n_vertices").unwrap_or(0).max(0) as usize;
    let vdim = n.get_i64("vertex_dim").unwrap_or(3).max(1) as usize;
    let rat = n.get_bool("rational");

    let raw_v = node_floats(n.get_i64("bspline_vertices").and_then(|i| nodes.get(&i).copied()), "vertices");
    let k_raw = node_floats(n.get_i64("knots").and_then(|i| nodes.get(&i).copied()), "knots");
    let m_raw = node_ints(n.get_i64("knot_mult").and_then(|i| nodes.get(&i).copied()), "mult");
    let (knots, multiplicities) = prune_knots(&k_raw, &m_raw);

    let is_pcurve = vdim == 2 || (vdim == 3 && rat && host_surface_index.is_some());
    let mut poles = Vec::new();
    let mut poles_2d = Vec::new();
    let mut weights = if rat { Some(Vec::with_capacity(nc)) } else { None };

    for i in 0..nc {
        let base = i * vdim;
        if is_pcurve {
            if base + 1 >= raw_v.len() {
                poles_2d.push([0.0, 0.0]);
                if let Some(ref mut ws) = weights {
                    ws.push(1.0);
                }
                continue;
            }
            if rat && vdim >= 3 && base + 2 < raw_v.len() {
                let w = if raw_v[base + 2] != 0.0 { raw_v[base + 2] } else { 1.0 };
                poles_2d.push([raw_v[base] / w, raw_v[base + 1] / w]);
                if let Some(ref mut ws) = weights {
                    ws.push(w);
                }
            } else {
                poles_2d.push([raw_v[base], raw_v[base + 1]]);
                if let Some(ref mut ws) = weights {
                    ws.push(1.0);
                }
            }
        } else {
            if base + 2 >= raw_v.len() {
                poles.push([0.0, 0.0, 0.0]);
                if let Some(ref mut ws) = weights {
                    ws.push(1.0);
                }
                continue;
            }
            if rat && vdim >= 4 && base + 3 < raw_v.len() {
                let w = if raw_v[base + 3] != 0.0 { raw_v[base + 3] } else { 1.0 };
                poles.push([raw_v[base] / w, raw_v[base + 1] / w, raw_v[base + 2] / w]);
                if let Some(ref mut ws) = weights {
                    ws.push(w);
                }
            } else {
                poles.push([raw_v[base], raw_v[base + 1], raw_v[base + 2]]);
                if let Some(ref mut ws) = weights {
                    ws.push(1.0);
                }
            }
        }
    }

    NurbsCurve {
        degree: n.get_i64("degree").unwrap_or(1).max(1) as usize,
        poles,
        knots,
        multiplicities,
        weights,
        periodic: n.get_bool("periodic"),
        closed: n.get_bool("closed"),
        rational: rat,
        vertex_dim: vdim,
        is_pcurve,
        poles_2d,
        host_surface_index,
    }
}

fn extract_surface(
    s_idx: Option<i64>,
    nodes: &BTreeMap<i64, &XtNode>,
    surf_cache: &mut BTreeMap<i64, SurfaceData>,
    curve_cache: &mut BTreeMap<i64, CurveData>,
    seen: &mut BTreeSet<i64>,
    warnings: &mut Vec<ConversionWarning>,
) -> SurfaceData {
    let Some(idx) = s_idx else {
        return SurfaceData {
            sense: Sense::Plus,
            kind: SurfaceKind::Unsupported("missing".into()),
        };
    };
    if let Some(cached) = surf_cache.get(&idx) {
        return cached.clone();
    }
    if !seen.insert(idx) {
        return SurfaceData {
            sense: Sense::Plus,
            kind: SurfaceKind::Unsupported("cyclic_surface".into()),
        };
    }
    let Some(&s) = nodes.get(&idx) else {
        return SurfaceData {
            sense: Sense::Plus,
            kind: SurfaceKind::Unsupported("unknown".into()),
        };
    };

    let sense = Sense::from_xt(s.get_first("sense"));
    let kind = match s.type_id {
        50 => SurfaceKind::Plane {
            origin: safe_vec3(s.get_vec3("pvec"), [0.0, 0.0, 0.0]),
            normal: safe_vec3(s.get_vec3("normal"), [0.0, 0.0, 1.0]),
            x_axis: safe_vec3(s.get_vec3("x_axis"), [1.0, 0.0, 0.0]),
        },
        51 => SurfaceKind::Cylinder {
            origin: safe_vec3(s.get_vec3("pvec"), [0.0, 0.0, 0.0]),
            axis: safe_vec3(s.get_vec3("axis"), [0.0, 0.0, 1.0]),
            x_axis: safe_vec3(s.get_vec3("x_axis"), [1.0, 0.0, 0.0]),
            radius: safe_float(s.get_f64("radius"), 0.0),
        },
        52 => SurfaceKind::Cone {
            origin: safe_vec3(s.get_vec3("pvec"), [0.0, 0.0, 0.0]),
            axis: safe_vec3(s.get_vec3("axis"), [0.0, 0.0, 1.0]),
            x_axis: safe_vec3(s.get_vec3("x_axis"), [1.0, 0.0, 0.0]),
            radius: safe_float(s.get_f64("radius"), 0.0),
            sin_half_angle: safe_float(s.get_f64("sin_half_angle"), 0.0),
            cos_half_angle: safe_float(s.get_f64("cos_half_angle"), 1.0),
        },
        53 => SurfaceKind::Sphere {
            centre: safe_vec3(s.get_vec3("centre"), [0.0, 0.0, 0.0]),
            axis: safe_vec3(s.get_vec3("axis"), [0.0, 0.0, 1.0]),
            x_axis: safe_vec3(s.get_vec3("x_axis"), [1.0, 0.0, 0.0]),
            radius: safe_float(s.get_f64("radius"), 0.0),
        },
        54 => SurfaceKind::Torus {
            centre: safe_vec3(s.get_vec3("centre"), [0.0, 0.0, 0.0]),
            axis: safe_vec3(s.get_vec3("axis"), [0.0, 0.0, 1.0]),
            x_axis: safe_vec3(s.get_vec3("x_axis"), [1.0, 0.0, 0.0]),
            major_radius: safe_float(s.get_f64("major_radius"), 0.0),
            minor_radius: safe_float(s.get_f64("minor_radius"), 0.0),
        },
        124 => {
            if let Some(nurbs_idx) = s.get_i64("nurbs") {
                if let Some(&nn) = nodes.get(&nurbs_idx) {
                    if nn.type_id == 126 {
                        SurfaceKind::Nurbs(Box::new(extract_nurbs_surface(nn, nodes)))
                    } else {
                        SurfaceKind::Unsupported("b_surface".into())
                    }
                } else {
                    SurfaceKind::Unsupported("b_surface".into())
                }
            } else {
                SurfaceKind::Unsupported("b_surface".into())
            }
        }
        126 => SurfaceKind::Nurbs(Box::new(extract_nurbs_surface(s, nodes))),
        67 => {
            let sec_idx = s.get_i64("section");
            let prof = extract_curve(sec_idx, nodes, curve_cache, surf_cache, warnings);
            SurfaceKind::Swept {
                direction: safe_vec3(s.get_vec3("sweep"), [0.0, 0.0, 1.0]),
                scale: safe_float(s.get_f64("scale"), 1.0),
                profile_index: sec_idx,
                profile: Some(Box::new(prof)),
            }
        }
        68 => {
            let prof_idx = s.get_i64("profile");
            let prof = extract_curve(prof_idx, nodes, curve_cache, surf_cache, warnings);
            SurfaceKind::Spun {
                base: safe_vec3(s.get_vec3("base"), [0.0, 0.0, 0.0]),
                axis: safe_vec3(s.get_vec3("axis"), [0.0, 0.0, 1.0]),
                x_axis: safe_vec3(s.get_vec3("x_axis"), [1.0, 0.0, 0.0]),
                profile_index: prof_idx,
                profile: Some(Box::new(prof)),
            }
        }
        60 => {
            let base_idx = s.get_i64("surface");
            let base = extract_surface(base_idx, nodes, surf_cache, curve_cache, seen, warnings);
            SurfaceKind::Offset {
                offset: safe_float(s.get_f64("offset"), 0.0),
                base_surface_index: base_idx,
                base_surface: Some(Box::new(base)),
            }
        }
        _ => {
            let nm = s.name.to_lowercase();
            warnings.push(ConversionWarning {
                code: "unsupported_surface".into(),
                subject: format!("surface #{idx}"),
                message: format!("Unsupported XT surface type {} ({})", s.name, s.type_id),
            });
            SurfaceKind::Unsupported(nm)
        }
    };

    let out = SurfaceData { sense, kind };
    surf_cache.insert(idx, out.clone());
    out
}

fn extract_curve(
    c_idx: Option<i64>,
    nodes: &BTreeMap<i64, &XtNode>,
    curve_cache: &mut BTreeMap<i64, CurveData>,
    surf_cache: &mut BTreeMap<i64, SurfaceData>,
    warnings: &mut Vec<ConversionWarning>,
) -> CurveData {
    let Some(root_idx) = c_idx else {
        return CurveData {
            sense: Sense::Plus,
            kind: CurveKind::Unsupported("missing".into()),
            sp_surface: None,
        };
    };
    if let Some(cached) = curve_cache.get(&root_idx) {
        return cached.clone();
    }

    let mut sp_surface_index: Option<i64> = None;
    let mut cur_idx = Some(root_idx);
    let mut seen = BTreeSet::new();

    while let Some(idx) = cur_idx {
        if !seen.insert(idx) {
            break;
        }
        let Some(&c) = nodes.get(&idx) else {
            cur_idx = None;
            break;
        };
        match c.type_id {
            133 => cur_idx = c.get_i64("basis_curve"),
            137 => {
                if sp_surface_index.is_none() {
                    sp_surface_index = c.get_i64("surface");
                }
                cur_idx = c.get_i64("b_curve").or_else(|| c.get_i64("original"));
            }
            134 => cur_idx = c.get_i64("nurbs"),
            _ => break,
        }
    }

    let Some(final_idx) = cur_idx else {
        let out = CurveData {
            sense: Sense::Plus,
            kind: CurveKind::Unsupported("unknown".into()),
            sp_surface: None,
        };
        curve_cache.insert(root_idx, out.clone());
        return out;
    };
    let Some(&c) = nodes.get(&final_idx) else {
        let out = CurveData {
            sense: Sense::Plus,
            kind: CurveKind::Unsupported("unknown".into()),
            sp_surface: None,
        };
        curve_cache.insert(root_idx, out.clone());
        return out;
    };

    let sense = Sense::from_xt(c.get_first("sense"));
    let kind = match c.type_id {
        30 => CurveKind::Line {
            origin: safe_vec3(c.get_vec3("pvec"), [0.0, 0.0, 0.0]),
            dir: safe_vec3(c.get_vec3("direction"), [1.0, 0.0, 0.0]),
        },
        31 => CurveKind::Circle {
            centre: safe_vec3(c.get_vec3("centre"), [0.0, 0.0, 0.0]),
            normal: safe_vec3(c.get_vec3("normal"), [0.0, 0.0, 1.0]),
            x_axis: safe_vec3(c.get_vec3("x_axis"), [1.0, 0.0, 0.0]),
            radius: safe_float(c.get_f64("radius"), 0.0),
        },
        32 => CurveKind::Ellipse {
            centre: safe_vec3(c.get_vec3("centre"), [0.0, 0.0, 0.0]),
            normal: safe_vec3(c.get_vec3("normal"), [0.0, 0.0, 1.0]),
            x_axis: safe_vec3(c.get_vec3("x_axis"), [1.0, 0.0, 0.0]),
            major_radius: safe_float(c.get_f64("major_radius"), 0.0),
            minor_radius: safe_float(c.get_f64("minor_radius"), 0.0),
        },
        136 => CurveKind::Nurbs(Box::new(extract_nurbs_curve(c, nodes, sp_surface_index))),
        38 => {
            let mut pts: Vec<[f64; 3]> = Vec::new();
            let append_hvec = |nid: Option<i64>, pts: &mut Vec<[f64; 3]>| {
                let Some(id) = nid else { return };
                let Some(&hn) = nodes.get(&id) else { return };
                let Some(vals) = hn.field("hvec") else { return };
                for v in vals {
                    let pt = safe_vec3(v.as_vec3(), [0.0, 0.0, 0.0]);
                    let distinct = match pts.last() {
                        Some(last) => {
                            let dx = pt[0] - last[0];
                            let dy = pt[1] - last[1];
                            let dz = pt[2] - last[2];
                            dx * dx + dy * dy + dz * dz > 1e-24
                        }
                        None => true,
                    };
                    if distinct {
                        pts.push(pt);
                    }
                }
            };
            append_hvec(c.get_i64("start"), &mut pts);
            append_hvec(c.get_i64("chart"), &mut pts);
            append_hvec(c.get_i64("end"), &mut pts);
            CurveKind::Intersection { points: pts }
        }
        _ => {
            let nm = c.name.to_lowercase();
            warnings.push(ConversionWarning {
                code: "unsupported_curve".into(),
                subject: format!("curve #{final_idx}"),
                message: format!("Unsupported XT curve type {} ({})", c.name, c.type_id),
            });
            CurveKind::Unsupported(nm)
        }
    };

    let sp_surface = sp_surface_index.map(|sidx| {
        let mut s_seen = BTreeSet::new();
        extract_surface(Some(sidx), nodes, surf_cache, curve_cache, &mut s_seen, warnings)
    });

    let out = CurveData {
        sense,
        kind,
        sp_surface,
    };
    curve_cache.insert(root_idx, out.clone());
    out
}

pub fn extract_brep(parsed: &ParseResult) -> BrepModel {
    let mut nodes: BTreeMap<i64, &XtNode> = BTreeMap::new();
    for n in &parsed.nodes {
        nodes.insert(n.index, n);
    }

    let mut warnings = Vec::new();
    let mut vertices = BTreeMap::new();
    for &n in nodes.values() {
        if n.type_id == 18 {
            let pvec = n
                .get_i64("point")
                .and_then(|pi| nodes.get(&pi).copied())
                .and_then(|pn| pn.get_vec3("pvec"));
            vertices.insert(
                n.index,
                Vertex {
                    index: n.index,
                    node_id: n.get_i64("node_id").unwrap_or(0),
                    point: safe_vec3(pvec, [0.0, 0.0, 0.0]),
                    tolerance: safe_float(n.get_f64("tolerance"), 0.0),
                },
            );
        }
    }

    let mut surfaces = BTreeMap::new();
    let mut curves = BTreeMap::new();
    let mut edges = BTreeMap::new();

    for &n in nodes.values() {
        if n.type_id != 16 {
            continue;
        }
        let mut c_idx = n.get_i64("curve").filter(|&i| i != 0);
        if c_idx.is_none() {
            let mut fin_idx = n.get_i64("halfedge").filter(|&i| i != 0);
            let mut visited = BTreeSet::new();
            while let Some(fi) = fin_idx {
                if !visited.insert(fi) {
                    break;
                }
                let Some(&fn_node) = nodes.get(&fi) else {
                    break;
                };
                let fc = fn_node.get_i64("curve").filter(|&i| i != 0);
                if fc.is_some() {
                    c_idx = fc;
                    break;
                }
                fin_idx = fn_node.get_i64("other").filter(|&i| i != 0);
            }
        }
        let c_data = extract_curve(c_idx, &nodes, &mut curves, &mut surfaces, &mut warnings);
        edges.insert(
            n.index,
            Edge {
                index: n.index,
                node_id: n.get_i64("node_id").unwrap_or(0),
                sense: c_data.sense,
                tolerance: safe_float(n.get_f64("tolerance"), 0.0),
                curve_index: c_idx,
                curve: c_data,
            },
        );
    }

    let mut faces = BTreeMap::new();
    for &n in nodes.values() {
        if n.type_id != 14 {
            continue;
        }
        let s_idx = n.get_i64("surface");
        let mut s_seen = BTreeSet::new();
        let s_data = extract_surface(
            s_idx,
            &nodes,
            &mut surfaces,
            &mut curves,
            &mut s_seen,
            &mut warnings,
        );
        let face_sense = Sense::from_xt(n.get_first("sense"));
        let surf_sense = s_data.sense;
        let combined_sense = if face_sense == surf_sense {
            Sense::Plus
        } else {
            Sense::Minus
        };

        let mut loops = Vec::new();
        let mut l_idx = n.get_i64("loop");
        let mut seen_l = BTreeSet::new();
        while let Some(li) = l_idx {
            if !seen_l.insert(li) {
                break;
            }
            let Some(&ln) = nodes.get(&li) else {
                break;
            };
            let mut coedges = Vec::new();
            let start_fin = ln.get_i64("halfedge");
            let mut cur_fin = start_fin;
            let mut seen_f = BTreeSet::new();
            while let Some(fi) = cur_fin {
                if !seen_f.insert(fi) {
                    break;
                }
                let Some(&fn_node) = nodes.get(&fi) else {
                    break;
                };
                let fwd_idx = fn_node.get_i64("forward");
                let bwd_idx = fn_node.get_i64("backward");
                let start_vertex = bwd_idx
                    .and_then(|bi| nodes.get(&bi).copied())
                    .and_then(|bn| bn.get_i64("vertex"));
                if let Some(e_idx) = fn_node.get_i64("edge") {
                    coedges.push(Coedge {
                        edge_index: e_idx,
                        sense: Sense::from_xt(fn_node.get_first("sense")),
                        start_vertex,
                        end_vertex: fn_node.get_i64("vertex"),
                        fin_index: fi,
                    });
                }
                cur_fin = fwd_idx;
                if cur_fin == start_fin {
                    break;
                }
            }
            loops.push(Loop { index: li, coedges });
            l_idx = ln.get_i64("next");
        }

        faces.insert(
            n.index,
            Face {
                index: n.index,
                node_id: n.get_i64("node_id").unwrap_or(0),
                shell_index: n.get_i64("shell"),
                sense: combined_sense,
                face_sense,
                surface_sense: surf_sense,
                surface_index: s_idx,
                surface: s_data,
                loops,
                tolerance: safe_float(n.get_f64("tolerance"), 0.0),
                appearance: Appearance::default(),
            },
        );
    }

    let (attributes, appearance_by_owner) = extract_attributes(&nodes);
    for face in faces.values_mut() {
        if let Some(app) = appearance_by_owner.get(&face.index) {
            face.appearance = app.clone();
        }
    }

    let mut bodies = Vec::new();
    for &n in nodes.values() {
        if n.type_id != 12 {
            continue;
        }
        let mut shell_indices = Vec::new();
        let mut sh = n.get_i64("shell");
        let mut seen_s = BTreeSet::new();
        while let Some(si) = sh {
            if !seen_s.insert(si) {
                break;
            }
            shell_indices.push(si);
            sh = nodes.get(&si).and_then(|sn| sn.get_i64("next"));
        }
        let face_count = faces
            .values()
            .filter(|f| f.shell_index.is_some_and(|si| seen_s.contains(&si)))
            .count();
        let appearance = appearance_by_owner.get(&n.index).cloned().unwrap_or_default();
        bodies.push(BodyInfo {
            index: n.index,
            node_id: n.get_i64("node_id").unwrap_or(0),
            body_type: n.get_i64("body_type"),
            shell_indices,
            face_count,
            name: appearance.name.clone(),
            appearance,
        });
    }

    let mut instances = Vec::new();
    let mut inst_by_asm: BTreeMap<i64, Vec<i64>> = BTreeMap::new();
    for &n in nodes.values() {
        if n.type_id != 11 {
            continue;
        }
        let tf_idx = n.get_i64("transform").filter(|&i| i != 0);
        let transform = tf_idx
            .and_then(|ti| nodes.get(&ti).copied())
            .map(Transform3D::from_xt);
        let asm_idx = n.get_i64("assembly").filter(|&i| i != 0);
        let part_idx = n.get_i64("part").filter(|&i| i != 0);
        let appearance = appearance_by_owner.get(&n.index).cloned().unwrap_or_default();
        if let Some(ai) = asm_idx {
            inst_by_asm.entry(ai).or_default().push(n.index);
        }
        instances.push(InstanceInfo {
            index: n.index,
            node_id: n.get_i64("node_id").unwrap_or(0),
            inst_type: n.get_i64("type"),
            assembly_index: asm_idx,
            part_index: part_idx,
            transform,
            name: appearance.name.clone(),
            appearance,
        });
    }

    let mut assemblies = Vec::new();
    for &n in nodes.values() {
        if n.type_id != 10 {
            continue;
        }
        let mut instance_indices = Vec::new();
        let mut seen_i = BTreeSet::new();
        let mut cur = n.get_i64("sub_instance").filter(|&i| i != 0);
        while let Some(ii) = cur {
            if !seen_i.insert(ii) {
                break;
            }
            if nodes.get(&ii).is_some_and(|in_node| in_node.type_id == 11) {
                instance_indices.push(ii);
                cur = nodes[&ii].get_i64("next_in_part").filter(|&i| i != 0);
            } else {
                break;
            }
        }
        if let Some(extra) = inst_by_asm.get(&n.index) {
            for &ii in extra {
                if seen_i.insert(ii) {
                    instance_indices.push(ii);
                }
            }
        }
        let appearance = appearance_by_owner.get(&n.index).cloned().unwrap_or_default();
        assemblies.push(AssemblyInfo {
            index: n.index,
            node_id: n.get_i64("node_id").unwrap_or(0),
            name: appearance.name.clone(),
            appearance,
            instance_indices,
        });
    }

    BrepModel {
        header: parsed.header.clone(),
        bodies,
        vertices,
        edges,
        faces,
        surfaces,
        curves,
        assemblies,
        instances,
        attributes,
        warnings,
    }
}

fn decode_bytes_string(vals: &[XtValue]) -> String {
    let bytes: Vec<u8> = vals
        .iter()
        .filter_map(|v| match v {
            XtValue::Byte(b) => Some(*b),
            XtValue::Int(i) => Some((*i & 0xFF) as u8),
            _ => None,
        })
        .take_while(|&b| b != 0)
        .collect();
    String::from_utf8_lossy(&bytes).into_owned()
}

fn extract_attributes(
    nodes: &BTreeMap<i64, &XtNode>,
) -> (Vec<Attribute>, BTreeMap<i64, Appearance>) {
    let mut owner_from_chain: BTreeMap<i64, i64> = BTreeMap::new();
    for &n in nodes.values() {
        if !matches!(n.type_id, 10 | 11 | 12 | 13 | 14 | 15 | 16 | 18 | 29) {
            continue;
        }
        let mut cur = n.get_i64("attributes_groups").filter(|&i| i != 0);
        let mut seen = BTreeSet::new();
        while let Some(ai) = cur {
            if !seen.insert(ai) {
                break;
            }
            let Some(&an) = nodes.get(&ai) else {
                break;
            };
            owner_from_chain.entry(ai).or_insert(n.index);
            cur = an.get_i64("next").filter(|&i| i != 0);
        }
    }

    let mut all_attrs = Vec::new();
    let mut app_by_owner: BTreeMap<i64, Appearance> = BTreeMap::new();

    for &n in nodes.values() {
        if n.type_id != 81 {
            continue;
        }
        let def_name = n
            .get_i64("definition")
            .and_then(|di| nodes.get(&di).copied())
            .and_then(|dn| dn.get_i64("identifier"))
            .and_then(|ii| nodes.get(&ii).copied())
            .and_then(|idn| idn.field("string"))
            .map(decode_bytes_string)
            .unwrap_or_else(|| "unknown".to_string());

        let mut values = Vec::new();
        if let Some(field_refs) = n.field("fields") {
            for fref in field_refs {
                let Some(f_idx) = fref.as_i64() else {
                    continue;
                };
                let Some(&vn) = nodes.get(&f_idx) else {
                    continue;
                };
                match vn.type_id {
                    84 => {
                        if let Some(raw) = vn.field("values") {
                            values.push(AttributeValue::Text(decode_bytes_string(raw)));
                        }
                    }
                    102 => {
                        if let Some(raw) = vn.field("string") {
                            values.push(AttributeValue::Text(decode_bytes_string(raw)));
                        }
                    }
                    98 => {
                        if let Some(raw) = vn.field("values") {
                            let units: Vec<u16> = raw
                                .iter()
                                .filter_map(XtValue::as_i64)
                                .map(|i| (i & 0xFFFF) as u16)
                                .take_while(|&u| u != 0)
                                .collect();
                            values.push(AttributeValue::Text(String::from_utf16_lossy(&units)));
                        }
                    }
                    82 => {
                        let ints: Vec<i64> = vn
                            .field("values")
                            .map(|vals| vals.iter().filter_map(XtValue::as_i64).collect())
                            .unwrap_or_default();
                        values.push(AttributeValue::Ints(ints));
                    }
                    83 => {
                        let reals: Vec<f64> = vn
                            .field("values")
                            .map(|vals| vals.iter().filter_map(XtValue::as_f64).collect())
                            .unwrap_or_default();
                        values.push(AttributeValue::Reals(reals));
                    }
                    85 | 86 | 87 | 89 => {
                        let vecs: Vec<[f64; 3]> = vn
                            .field("values")
                            .map(|vals| vals.iter().filter_map(XtValue::as_vec3).collect())
                            .unwrap_or_default();
                        values.push(AttributeValue::Vec3s(vecs));
                    }
                    _ => {}
                }
            }
        }

        let owner_index = n
            .get_i64("owner")
            .filter(|&i| i != 0)
            .or_else(|| owner_from_chain.get(&n.index).copied());
        let attr = Attribute {
            index: n.index,
            name: def_name,
            owner_index,
            values,
        };
        if let Some(oi) = owner_index {
            app_by_owner.entry(oi).or_default().apply_attribute(&attr);
        }
        all_attrs.push(attr);
    }

    (all_attrs, app_by_owner)
}

pub fn transform_curve_data(curve: &CurveData, tf: &Transform3D) -> CurveData {
    let sp_surface = curve
        .sp_surface
        .as_ref()
        .map(|s| transform_surface_data(s, tf));
    let flip_sense = tf.det() < 0.0;
    let kind = match &curve.kind {
        CurveKind::Line { origin, dir } => CurveKind::Line {
            origin: tf.apply_point(*origin),
            dir: tf.apply_dir(*dir),
        },
        CurveKind::Circle {
            centre,
            normal,
            x_axis,
            radius,
        } => {
            let n_dir = tf.apply_dir(*normal);
            CurveKind::Circle {
                centre: tf.apply_point(*centre),
                normal: if flip_sense {
                    [-n_dir[0], -n_dir[1], -n_dir[2]]
                } else {
                    n_dir
                },
                x_axis: tf.apply_dir(*x_axis),
                radius: radius * tf.scale.abs(),
            }
        }
        CurveKind::Ellipse {
            centre,
            normal,
            x_axis,
            major_radius,
            minor_radius,
        } => {
            let n_dir = tf.apply_dir(*normal);
            CurveKind::Ellipse {
                centre: tf.apply_point(*centre),
                normal: if flip_sense {
                    [-n_dir[0], -n_dir[1], -n_dir[2]]
                } else {
                    n_dir
                },
                x_axis: tf.apply_dir(*x_axis),
                major_radius: major_radius * tf.scale.abs(),
                minor_radius: minor_radius * tf.scale.abs(),
            }
        }
        CurveKind::Nurbs(nc) => {
            let mut copy = (**nc).clone();
            if !copy.is_pcurve {
                for p in &mut copy.poles {
                    *p = tf.apply_point(*p);
                }
            }
            CurveKind::Nurbs(Box::new(copy))
        }
        CurveKind::Intersection { points } => CurveKind::Intersection {
            points: points.iter().map(|&p| tf.apply_point(p)).collect(),
        },
        CurveKind::Unsupported(s) => CurveKind::Unsupported(s.clone()),
    };
    CurveData {
        sense: curve.sense,
        kind,
        sp_surface,
    }
}

pub fn transform_surface_data(surf: &SurfaceData, tf: &Transform3D) -> SurfaceData {
    let sense = surf.sense;
    let kind = match &surf.kind {
        SurfaceKind::Plane {
            origin,
            normal,
            x_axis,
        } => SurfaceKind::Plane {
            origin: tf.apply_point(*origin),
            normal: tf.apply_dir(*normal),
            x_axis: tf.apply_dir(*x_axis),
        },
        SurfaceKind::Cylinder {
            origin,
            axis,
            x_axis,
            radius,
        } => SurfaceKind::Cylinder {
            origin: tf.apply_point(*origin),
            axis: tf.apply_dir(*axis),
            x_axis: tf.apply_dir(*x_axis),
            radius: radius * tf.scale.abs(),
        },
        SurfaceKind::Cone {
            origin,
            axis,
            x_axis,
            radius,
            sin_half_angle,
            cos_half_angle,
        } => SurfaceKind::Cone {
            origin: tf.apply_point(*origin),
            axis: tf.apply_dir(*axis),
            x_axis: tf.apply_dir(*x_axis),
            radius: radius * tf.scale.abs(),
            sin_half_angle: *sin_half_angle,
            cos_half_angle: *cos_half_angle,
        },
        SurfaceKind::Sphere {
            centre,
            axis,
            x_axis,
            radius,
        } => SurfaceKind::Sphere {
            centre: tf.apply_point(*centre),
            axis: tf.apply_dir(*axis),
            x_axis: tf.apply_dir(*x_axis),
            radius: radius * tf.scale.abs(),
        },
        SurfaceKind::Torus {
            centre,
            axis,
            x_axis,
            major_radius,
            minor_radius,
        } => SurfaceKind::Torus {
            centre: tf.apply_point(*centre),
            axis: tf.apply_dir(*axis),
            x_axis: tf.apply_dir(*x_axis),
            major_radius: major_radius * tf.scale.abs(),
            minor_radius: minor_radius * tf.scale.abs(),
        },
        SurfaceKind::Nurbs(ns) => {
            let mut copy = (**ns).clone();
            for row in &mut copy.poles {
                for p in row {
                    *p = tf.apply_point(*p);
                }
            }
            SurfaceKind::Nurbs(Box::new(copy))
        }
        SurfaceKind::Swept {
            direction,
            scale,
            profile_index,
            profile,
        } => SurfaceKind::Swept {
            direction: tf.apply_dir(*direction),
            scale: *scale * tf.scale,
            profile_index: *profile_index,
            profile: profile
                .as_ref()
                .map(|c| Box::new(transform_curve_data(c, tf))),
        },
        SurfaceKind::Spun {
            base,
            axis,
            x_axis,
            profile_index,
            profile,
        } => SurfaceKind::Spun {
            base: tf.apply_point(*base),
            axis: tf.apply_dir(*axis),
            x_axis: tf.apply_dir(*x_axis),
            profile_index: *profile_index,
            profile: profile
                .as_ref()
                .map(|c| Box::new(transform_curve_data(c, tf))),
        },
        SurfaceKind::Offset {
            offset,
            base_surface_index,
            base_surface,
        } => SurfaceKind::Offset {
            offset: offset * tf.scale.abs(),
            base_surface_index: *base_surface_index,
            base_surface: base_surface
                .as_ref()
                .map(|s| Box::new(transform_surface_data(s, tf))),
        },
        SurfaceKind::Unsupported(s) => SurfaceKind::Unsupported(s.clone()),
    };
    SurfaceData { sense, kind }
}
