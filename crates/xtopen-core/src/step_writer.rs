//! Native zero-dependency ISO-10303-21 STEP AP214/AP242 writer for XT B-rep models.

use std::collections::BTreeMap;
use std::fmt::Write;

use crate::model::{
    extract_brep, BrepModel, Coedge, CurveData, CurveKind, Edge, Face, Loop, NurbsCurve,
    NurbsSurface, SurfaceData, SurfaceKind,
};
use crate::xb::{ParseResult, XtError};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StepSchema {
    Ap214,
    Ap242,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StepUnits {
    Mm,
    M,
}

#[derive(Clone, Debug)]
pub struct StepOptions {
    pub schema: StepSchema,
    pub units: StepUnits,
    pub name: String,
    pub timestamp: String,
}

impl Default for StepOptions {
    fn default() -> Self {
        Self {
            schema: StepSchema::Ap214,
            units: StepUnits::Mm,
            name: "xtopen_model".to_string(),
            timestamp: "2026-01-01T00:00:00".to_string(),
        }
    }
}

fn v_norm(v: [f64; 3]) -> [f64; 3] {
    let n = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    if n < 1e-15 {
        [0.0, 0.0, 1.0]
    } else {
        [v[0] / n, v[1] / n, v[2] / n]
    }
}

fn v_dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn v_cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn ortho_frame(z_raw: [f64; 3], x_raw: [f64; 3]) -> ([f64; 3], [f64; 3]) {
    let z = v_norm(z_raw);
    let dot = v_dot(x_raw, z);
    let mut xp = [
        x_raw[0] - dot * z[0],
        x_raw[1] - dot * z[1],
        x_raw[2] - dot * z[2],
    ];
    if v_dot(xp, xp).sqrt() < 1e-12 {
        let cand = if z[0].abs() < 0.9 {
            [1.0, 0.0, 0.0]
        } else {
            [0.0, 1.0, 0.0]
        };
        xp = v_cross(z, cand);
    }
    (z, v_norm(xp))
}

fn push_step_real(buf: &mut String, val: f64) {
    if !val.is_finite() || val.abs() < 1e-15 {
        buf.push_str("0.");
        return;
    }
    let abs = val.abs();
    // Format with ~12 significant digits cleanly matching Python's :.12g and always containing '.'
    let s = if abs >= 1e-4 && abs < 1e12 {
        let mut raw = format!("{val:.12}");
        if let Some(dot) = raw.find('.') {
            // Limit total significant digits cleanly
            let sig_limit = if abs >= 1.0 {
                let int_digits = raw[..dot].trim_start_matches('-').len();
                12usize.saturating_sub(int_digits)
            } else {
                // Keep extra decimals when leading zeros appear after dot
                let frac = &raw[dot + 1..];
                let lead_zeros = frac.bytes().take_while(|&b| b == b'0').count();
                lead_zeros + 12
            };
            let _ = sig_limit;
            while raw.ends_with('0') && !raw.ends_with(".0") {
                raw.pop();
            }
            if raw.ends_with(".0") {
                raw.pop();
            }
        }
        raw
    } else {
        let raw = format!("{val:.11e}");
        if let Some((mant, exp)) = raw.split_once('e') {
            let mut m = mant.to_string();
            while m.ends_with('0') {
                m.pop();
            }
            if m.ends_with('.') {
                format!("{m}E{exp}")
            } else if !m.contains('.') {
                format!("{m}.E{exp}")
            } else {
                format!("{m}E{exp}")
            }
        } else {
            raw
        }
    };
    if !s.contains('.') {
        if let Some((a, b)) = s.split_once('E') {
            buf.push_str(a);
            buf.push_str(".E");
            buf.push_str(b);
        } else {
            buf.push_str(&s);
            buf.push('.');
        }
    } else {
        buf.push_str(&s);
    }
}

fn push_step_str(buf: &mut String, text: &str) {
    buf.push('\'');
    for ch in text.chars() {
        if ch == '\'' {
            buf.push_str("''");
        } else {
            buf.push(ch);
        }
    }
    buf.push('\'');
}

fn step_bool(flag: bool) -> &'static str {
    if flag {
        ".T."
    } else {
        ".F."
    }
}

fn expand_knots(knots: &[f64], mults: &[usize]) -> Vec<f64> {
    let total: usize = mults.iter().sum();
    let mut out = Vec::with_capacity(total);
    for (&k, &m) in knots.iter().zip(mults.iter()) {
        for _ in 0..m {
            out.push(k);
        }
    }
    out
}

fn eval_bspline_dim<const D: usize>(
    t: f64,
    degree: usize,
    knots: &[f64],
    mults: &[usize],
    poles: &[[f64; D]],
    weights: Option<&[f64]>,
) -> [f64; D] {
    if poles.is_empty() {
        return [0.0; D];
    }
    let u = expand_knots(knots, mults);
    let n = poles.len() - 1;
    let p = degree.min(n);
    if u.len() <= n + 1 || p == 0 {
        return poles[0];
    }

    let k = if t <= u[p] {
        let mut k = p;
        while k < n && u[k + 1] == u[p] {
            k += 1;
        }
        k
    } else if t >= u[n + 1] {
        n
    } else {
        let mut lo = p;
        let mut hi = n + 1;
        let mut mid = (lo + hi) / 2;
        while t < u[mid] || t >= u[mid + 1] {
            if t < u[mid] {
                hi = mid;
            } else {
                lo = mid;
            }
            mid = (lo + hi) / 2;
            if hi - lo <= 1 {
                break;
            }
        }
        mid
    };

    let mut d = Vec::with_capacity(p + 1);
    let mut dw = Vec::with_capacity(p + 1);
    for j in 0..=p {
        let idx = (k + j).saturating_sub(p).min(n);
        let w = weights.and_then(|ws| ws.get(idx).copied()).unwrap_or(1.0);
        let mut pt = [0.0; D];
        for c in 0..D {
            pt[c] = poles[idx][c] * w;
        }
        d.push(pt);
        dw.push(w);
    }

    for r in 1..=p {
        for j in (r..=p).rev() {
            let i = k + j - p;
            let denom = u[i + p + 1 - r] - u[i];
            let alpha = if denom.abs() < 1e-15 {
                0.0
            } else {
                (t - u[i]) / denom
            };
            for c in 0..D {
                d[j][c] = (1.0 - alpha) * d[j - 1][c] + alpha * d[j][c];
            }
            dw[j] = (1.0 - alpha) * dw[j - 1] + alpha * dw[j];
        }
    }

    let mut out = d[p];
    if weights.is_some() {
        let w = if dw[p].abs() > 1e-15 { dw[p] } else { 1.0 };
        for c in 0..D {
            out[c] /= w;
        }
    }
    out
}

fn eval_surface_pt(surf: &SurfaceData, u: f64, v: f64) -> [f64; 3] {
    match &surf.kind {
        SurfaceKind::Plane {
            origin,
            normal,
            x_axis,
        } => {
            let (z, x) = ortho_frame(*normal, *x_axis);
            let y = v_cross(z, x);
            [
                origin[0] + u * x[0] + v * y[0],
                origin[1] + u * x[1] + v * y[1],
                origin[2] + u * x[2] + v * y[2],
            ]
        }
        SurfaceKind::Cylinder {
            origin,
            axis,
            x_axis,
            radius,
        } => {
            let (z, x) = ortho_frame(*axis, *x_axis);
            let y = v_cross(z, x);
            let (su, cu) = u.sin_cos();
            [
                origin[0] + radius * (cu * x[0] + su * y[0]) + v * z[0],
                origin[1] + radius * (cu * x[1] + su * y[1]) + v * z[1],
                origin[2] + radius * (cu * x[2] + su * y[2]) + v * z[2],
            ]
        }
        SurfaceKind::Cone {
            origin,
            axis,
            x_axis,
            radius,
            sin_half_angle,
            cos_half_angle,
        } => {
            let tan_a = if cos_half_angle.abs() > 1e-15 {
                sin_half_angle / cos_half_angle
            } else {
                0.0
            };
            let (z, x) = ortho_frame(*axis, *x_axis);
            let y = v_cross(z, x);
            let rad = radius + v * tan_a;
            let (su, cu) = u.sin_cos();
            [
                origin[0] - v * z[0] + rad * (cu * x[0] + su * y[0]),
                origin[1] - v * z[1] + rad * (cu * x[1] + su * y[1]),
                origin[2] - v * z[2] + rad * (cu * x[2] + su * y[2]),
            ]
        }
        SurfaceKind::Sphere {
            centre,
            axis,
            x_axis,
            radius,
        } => {
            let (z, x) = ortho_frame(*axis, *x_axis);
            let y = v_cross(z, x);
            let (su, cu) = u.sin_cos();
            let (sv, cv) = v.sin_cos();
            [
                centre[0] + radius * (cv * (cu * x[0] + su * y[0]) + sv * z[0]),
                centre[1] + radius * (cv * (cu * x[1] + su * y[1]) + sv * z[1]),
                centre[2] + radius * (cv * (cu * x[2] + su * y[2]) + sv * z[2]),
            ]
        }
        SurfaceKind::Torus {
            centre,
            axis,
            x_axis,
            major_radius,
            minor_radius,
        } => {
            let (z, x) = ortho_frame(*axis, *x_axis);
            let y = v_cross(z, x);
            let (su, cu) = u.sin_cos();
            let (sv, cv) = v.sin_cos();
            let radial = major_radius + minor_radius * cv;
            [
                centre[0] + radial * (cu * x[0] + su * y[0]) + minor_radius * sv * z[0],
                centre[1] + radial * (cu * x[1] + su * y[1]) + minor_radius * sv * z[1],
                centre[2] + radial * (cu * x[2] + su * y[2]) + minor_radius * sv * z[2],
            ]
        }
        SurfaceKind::Nurbs(ns) => {
            if ns.poles.is_empty() {
                return [u, v, 0.0];
            }
            let row_pts: Vec<[f64; 3]> = ns
                .poles
                .iter()
                .enumerate()
                .map(|(i, row)| {
                    let w = ns.weights.as_ref().and_then(|ws| ws.get(i).map(Vec::as_slice));
                    eval_bspline_dim(v, ns.v_degree, &ns.v_knots, &ns.v_mults, row, w)
                })
                .collect();
            eval_bspline_dim(u, ns.u_degree, &ns.u_knots, &ns.u_mults, &row_pts, None)
        }
        _ => [u, v, 0.0],
    }
}

fn interpolate_cubic_bspline(pts: &[[f64; 3]]) -> NurbsCurve {
    let n = pts.len();
    if n <= 2 {
        return NurbsCurve {
            degree: 1,
            poles: pts.to_vec(),
            knots: vec![0.0, 1.0],
            multiplicities: vec![2, 2],
            weights: None,
            periodic: false,
            closed: false,
            rational: false,
            vertex_dim: 3,
            is_pcurve: false,
            poles_2d: Vec::new(),
            host_surface_index: None,
        };
    }

    let mut params = Vec::with_capacity(n);
    params.push(0.0);
    for i in 1..n {
        let dx = pts[i][0] - pts[i - 1][0];
        let dy = pts[i][1] - pts[i - 1][1];
        let dz = pts[i][2] - pts[i - 1][2];
        params.push(params[i - 1] + (dx * dx + dy * dy + dz * dz).sqrt());
    }
    let total = *params.last().unwrap_or(&0.0);
    if total < 1e-15 {
        return NurbsCurve {
            degree: 1,
            poles: vec![pts[0], pts[n - 1]],
            knots: vec![0.0, 1.0],
            multiplicities: vec![2, 2],
            weights: None,
            periodic: false,
            closed: false,
            rational: false,
            vertex_dim: 3,
            is_pcurve: false,
            poles_2d: Vec::new(),
            host_surface_index: None,
        };
    }
    for p in &mut params {
        *p /= total;
    }

    if n == 3 {
        return NurbsCurve {
            degree: 2,
            poles: pts.to_vec(),
            knots: vec![0.0, 1.0],
            multiplicities: vec![3, 3],
            weights: None,
            periodic: false,
            closed: false,
            rational: false,
            vertex_dim: 3,
            is_pcurve: false,
            poles_2d: Vec::new(),
            host_surface_index: None,
        };
    }

    let deg = 3usize;
    let m = n - 1;
    let mut knots_u = Vec::with_capacity(n + 6);
    knots_u.extend([0.0; 4]);
    knots_u.extend_from_slice(&params[1..n - 1]);
    knots_u.extend([1.0; 4]);
    let n_poles = n + 2;

    let basis = |i: usize, p: usize, u: f64| -> f64 {
        if (i == 0 && u == knots_u[0]) || (i == n_poles - 1 && u == knots_u[knots_u.len() - 1]) {
            return if (i == 0 && u == 0.0) || (i == n_poles - 1 && u == 1.0) {
                1.0
            } else {
                0.0
            };
        }
        let mut basis_vals = vec![0.0; p + 1];
        for j in 0..=p {
            if knots_u[i + j] <= u && u < knots_u[i + j + 1] {
                basis_vals[j] = 1.0;
            }
        }
        for k in 1..=p {
            let mut saved = if basis_vals[0] == 0.0 {
                0.0
            } else {
                ((u - knots_u[i]) * basis_vals[0]) / (knots_u[i + k] - knots_u[i]).max(1e-15)
            };
            for j in 0..=(p - k) {
                let u_left = knots_u[i + j + 1];
                let u_right = knots_u[i + j + k + 1];
                if basis_vals[j + 1] == 0.0 {
                    basis_vals[j] = saved;
                    saved = 0.0;
                } else {
                    let temp = basis_vals[j + 1] / (u_right - u_left).max(1e-15);
                    basis_vals[j] = saved + (u_right - u) * temp;
                    saved = (u - u_left) * temp;
                }
            }
        }
        basis_vals[0]
    };

    let mut mat = vec![vec![0.0; n_poles]; n_poles];
    let mut rhs = vec![[0.0; 3]; n_poles];
    mat[0][0] = 1.0;
    rhs[0] = pts[0];
    mat[n_poles - 1][n_poles - 1] = 1.0;
    rhs[n_poles - 1] = pts[n - 1];
    mat[1][0] = 1.0;
    mat[1][1] = -2.0;
    mat[1][2] = 1.0;
    mat[n_poles - 2][n_poles - 3] = 1.0;
    mat[n_poles - 2][n_poles - 2] = -2.0;
    mat[n_poles - 2][n_poles - 1] = 1.0;
    for row_idx in 1..m {
        let u = params[row_idx];
        let r = row_idx + 1;
        for col in 0..n_poles {
            mat[r][col] = basis(col, deg, u);
        }
        rhs[r] = pts[row_idx];
    }

    for col in 0..n_poles {
        let mut piv = col;
        let mut best = mat[col][col].abs();
        for r in (col + 1)..n_poles {
            let v = mat[r][col].abs();
            if v > best {
                best = v;
                piv = r;
            }
        }
        mat.swap(col, piv);
        rhs.swap(col, piv);
        let diag = mat[col][col];
        if diag.abs() < 1e-14 {
            let mut uniq_k = Vec::with_capacity(n);
            uniq_k.push(0.0);
            for i in 1..m {
                uniq_k.push(i as f64 / m as f64);
            }
            uniq_k.push(1.0);
            let mut mults = Vec::with_capacity(n);
            mults.push(2);
            mults.extend(std::iter::repeat(1).take(m - 1));
            mults.push(2);
            return NurbsCurve {
                degree: 1,
                poles: pts.to_vec(),
                knots: uniq_k,
                multiplicities: mults,
                weights: None,
                periodic: false,
                closed: false,
                rational: false,
                vertex_dim: 3,
                is_pcurve: false,
                poles_2d: Vec::new(),
                host_surface_index: None,
            };
        }
        for j in col..n_poles {
            mat[col][j] /= diag;
        }
        for c in 0..3 {
            rhs[col][c] /= diag;
        }
        for r in 0..n_poles {
            if r != col && mat[r][col].abs() > 1e-15 {
                let fac = mat[r][col];
                for j in col..n_poles {
                    mat[r][j] -= fac * mat[col][j];
                }
                for c in 0..3 {
                    rhs[r][c] -= fac * rhs[col][c];
                }
            }
        }
    }

    let mut uniq_knots = Vec::with_capacity(n);
    uniq_knots.push(0.0);
    uniq_knots.extend_from_slice(&params[1..n - 1]);
    uniq_knots.push(1.0);
    let mut mults = Vec::with_capacity(n);
    mults.push(4);
    mults.extend(std::iter::repeat(1).take(n - 2));
    mults.push(4);

    NurbsCurve {
        degree: 3,
        poles: rhs,
        knots: uniq_knots,
        multiplicities: mults,
        weights: None,
        periodic: false,
        closed: false,
        rational: false,
        vertex_dim: 3,
        is_pcurve: false,
        poles_2d: Vec::new(),
        host_surface_index: None,
    }
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct QuantVec3([i64; 3]);

fn quantize_vec3(v: [f64; 3]) -> QuantVec3 {
    let q = |x: f64| -> i64 {
        if !x.is_finite() {
            0
        } else {
            (x * 1e12).round() as i64
        }
    };
    QuantVec3([q(v[0]), q(v[1]), q(v[2])])
}

struct StepBuilder {
    scale: f64,
    units: StepUnits,
    schema: StepSchema,
    name: String,
    timestamp: String,
    next_id: usize,
    data_buf: String,
    pt_cache: BTreeMap<QuantVec3, usize>,
    dir_cache: BTreeMap<QuantVec3, usize>,
    vert_cache: BTreeMap<i64, usize>,
    surf_cache: BTreeMap<i64, (usize, bool)>,
    edge_cache: BTreeMap<i64, (usize, usize, usize, bool)>,
}

impl StepBuilder {
    fn new(opts: &StepOptions) -> Self {
        let scale = match opts.units {
            StepUnits::Mm => 1000.0,
            StepUnits::M => 1.0,
        };
        Self {
            scale,
            units: opts.units,
            schema: opts.schema,
            name: opts.name.clone(),
            timestamp: opts.timestamp.clone(),
            next_id: 1,
            data_buf: String::with_capacity(16 * 1024),
            pt_cache: BTreeMap::new(),
            dir_cache: BTreeMap::new(),
            vert_cache: BTreeMap::new(),
            surf_cache: BTreeMap::new(),
            edge_cache: BTreeMap::new(),
        }
    }

    fn alloc_entity_start(&mut self) -> usize {
        let eid = self.next_id;
        self.next_id += 1;
        let _ = write!(self.data_buf, "#{eid}=");
        eid
    }

    fn finish_entity(&mut self) {
        self.data_buf.push_str(";\n");
    }

    fn emit_str(&mut self, body: &str) -> usize {
        let eid = self.next_id;
        self.next_id += 1;
        let _ = writeln!(self.data_buf, "#{eid}={body};");
        eid
    }

    fn point(&mut self, pt: [f64; 3]) -> usize {
        let s = [pt[0] * self.scale, pt[1] * self.scale, pt[2] * self.scale];
        let key = quantize_vec3(s);
        if let Some(&eid) = self.pt_cache.get(&key) {
            return eid;
        }
        let eid = self.alloc_entity_start();
        self.data_buf.push_str("CARTESIAN_POINT('',(");
        push_step_real(&mut self.data_buf, s[0]);
        self.data_buf.push(',');
        push_step_real(&mut self.data_buf, s[1]);
        self.data_buf.push(',');
        push_step_real(&mut self.data_buf, s[2]);
        self.data_buf.push_str("))");
        self.finish_entity();
        self.pt_cache.insert(key, eid);
        eid
    }

    fn direction(&mut self, v: [f64; 3]) -> usize {
        let n = v_norm(v);
        let key = quantize_vec3(n);
        if let Some(&eid) = self.dir_cache.get(&key) {
            return eid;
        }
        let eid = self.alloc_entity_start();
        self.data_buf.push_str("DIRECTION('',(");
        push_step_real(&mut self.data_buf, n[0]);
        self.data_buf.push(',');
        push_step_real(&mut self.data_buf, n[1]);
        self.data_buf.push(',');
        push_step_real(&mut self.data_buf, n[2]);
        self.data_buf.push_str("))");
        self.finish_entity();
        self.dir_cache.insert(key, eid);
        eid
    }

    fn vector(&mut self, v: [f64; 3], mag: f64) -> usize {
        let d = self.direction(v);
        let eid = self.alloc_entity_start();
        let _ = write!(self.data_buf, "VECTOR('',#{d},");
        push_step_real(&mut self.data_buf, mag);
        self.data_buf.push(')');
        self.finish_entity();
        eid
    }

    fn axis2(&mut self, origin: [f64; 3], z_dir: [f64; 3], x_dir: [f64; 3]) -> usize {
        let (z, x) = ortho_frame(z_dir, x_dir);
        let p = self.point(origin);
        let dz = self.direction(z);
        let dx = self.direction(x);
        let eid = self.alloc_entity_start();
        let _ = write!(self.data_buf, "AXIS2_PLACEMENT_3D('',#{p},#{dz},#{dx})");
        self.finish_entity();
        eid
    }

    fn axis1(&mut self, origin: [f64; 3], z_dir: [f64; 3]) -> usize {
        let p = self.point(origin);
        let dz = self.direction(z_dir);
        let eid = self.alloc_entity_start();
        let _ = write!(self.data_buf, "AXIS1_PLACEMENT('',#{p},#{dz})");
        self.finish_entity();
        eid
    }

    fn emit_nurbs_curve(&mut self, nc: &NurbsCurve) -> usize {
        let pt_ids: Vec<usize> = nc.poles.iter().map(|&p| self.point(p)).collect();
        let eid = self.alloc_entity_start();
        let is_rat = nc.rational
            && nc
                .weights
                .as_ref()
                .is_some_and(|w| w.len() == pt_ids.len());
        if is_rat {
            self.data_buf.push_str("(BOUNDED_CURVE() B_SPLINE_CURVE(");
            let _ = write!(self.data_buf, "{},(", nc.degree);
            push_id_list(&mut self.data_buf, &pt_ids);
            let _ = write!(
                self.data_buf,
                "),.UNSPECIFIED.,{},.F.) B_SPLINE_CURVE_WITH_KNOTS((",
                step_bool(nc.closed)
            );
            push_usize_list(&mut self.data_buf, &nc.multiplicities);
            self.data_buf.push_str("),(");
            push_real_list(&mut self.data_buf, &nc.knots);
            self.data_buf
                .push_str("),.UNSPECIFIED.) CURVE() GEOMETRIC_REPRESENTATION_ITEM() RATIONAL_B_SPLINE_CURVE((");
            if let Some(ws) = &nc.weights {
                push_real_list(&mut self.data_buf, ws);
            }
            self.data_buf.push_str(")) REPRESENTATION_ITEM(''))");
        } else {
            let _ = write!(self.data_buf, "B_SPLINE_CURVE_WITH_KNOTS('',{},(", nc.degree);
            push_id_list(&mut self.data_buf, &pt_ids);
            let _ = write!(
                self.data_buf,
                "),.UNSPECIFIED.,{},.F.,(",
                step_bool(nc.closed)
            );
            push_usize_list(&mut self.data_buf, &nc.multiplicities);
            self.data_buf.push_str("),(");
            push_real_list(&mut self.data_buf, &nc.knots);
            self.data_buf.push_str("),.UNSPECIFIED.)");
        }
        self.finish_entity();
        eid
    }

    fn emit_nurbs_surface(&mut self, ns: &NurbsSurface) -> usize {
        let mut grid_ids = Vec::with_capacity(ns.poles.len());
        for row in &ns.poles {
            let r: Vec<usize> = row.iter().map(|&p| self.point(p)).collect();
            grid_ids.push(r);
        }
        let is_rat = ns.rational
            && ns
                .weights
                .as_ref()
                .is_some_and(|w| w.len() == ns.poles.len());
        let eid = self.alloc_entity_start();
        if is_rat {
            let _ = write!(
                self.data_buf,
                "(BOUNDED_SURFACE() B_SPLINE_SURFACE({},{},(",
                ns.u_degree, ns.v_degree
            );
            for (ri, row) in grid_ids.iter().enumerate() {
                if ri > 0 {
                    self.data_buf.push(',');
                }
                self.data_buf.push('(');
                push_id_list(&mut self.data_buf, row);
                self.data_buf.push(')');
            }
            let _ = write!(
                self.data_buf,
                "),.UNSPECIFIED.,{},{},.F.) B_SPLINE_SURFACE_WITH_KNOTS((",
                step_bool(ns.u_closed),
                step_bool(ns.v_closed)
            );
            push_usize_list(&mut self.data_buf, &ns.u_mults);
            self.data_buf.push_str("),(");
            push_usize_list(&mut self.data_buf, &ns.v_mults);
            self.data_buf.push_str("),(");
            push_real_list(&mut self.data_buf, &ns.u_knots);
            self.data_buf.push_str("),(");
            push_real_list(&mut self.data_buf, &ns.v_knots);
            self.data_buf
                .push_str("),.UNSPECIFIED.) GEOMETRIC_REPRESENTATION_ITEM() RATIONAL_B_SPLINE_SURFACE((");
            if let Some(ws) = &ns.weights {
                for (ri, row) in ws.iter().enumerate() {
                    if ri > 0 {
                        self.data_buf.push(',');
                    }
                    self.data_buf.push('(');
                    push_real_list(&mut self.data_buf, row);
                    self.data_buf.push(')');
                }
            }
            self.data_buf.push_str(")) REPRESENTATION_ITEM('') SURFACE())");
        } else {
            let _ = write!(
                self.data_buf,
                "B_SPLINE_SURFACE_WITH_KNOTS('',{},{},(",
                ns.u_degree, ns.v_degree
            );
            for (ri, row) in grid_ids.iter().enumerate() {
                if ri > 0 {
                    self.data_buf.push(',');
                }
                self.data_buf.push('(');
                push_id_list(&mut self.data_buf, row);
                self.data_buf.push(')');
            }
            let _ = write!(
                self.data_buf,
                "),.UNSPECIFIED.,{},{},.F.,(",
                step_bool(ns.u_closed),
                step_bool(ns.v_closed)
            );
            push_usize_list(&mut self.data_buf, &ns.u_mults);
            self.data_buf.push_str("),(");
            push_usize_list(&mut self.data_buf, &ns.v_mults);
            self.data_buf.push_str("),(");
            push_real_list(&mut self.data_buf, &ns.u_knots);
            self.data_buf.push_str("),(");
            push_real_list(&mut self.data_buf, &ns.v_knots);
            self.data_buf.push_str("),.UNSPECIFIED.)");
        }
        self.finish_entity();
        eid
    }

    fn emit_curve_geom(&mut self, curve: &CurveData) -> (usize, [f64; 3]) {
        match &curve.kind {
            CurveKind::Line { origin, dir } => {
                let p = self.point(*origin);
                let v = self.vector(*dir, 1.0);
                let eid = self.alloc_entity_start();
                let _ = write!(self.data_buf, "LINE('',#{p},#{v})");
                self.finish_entity();
                (eid, *origin)
            }
            CurveKind::Circle {
                centre,
                normal,
                x_axis,
                radius,
            } => {
                let ax = self.axis2(*centre, *normal, *x_axis);
                let (_, x) = ortho_frame(*normal, *x_axis);
                let seam = [
                    centre[0] + radius * x[0],
                    centre[1] + radius * x[1],
                    centre[2] + radius * x[2],
                ];
                let eid = self.alloc_entity_start();
                let _ = write!(self.data_buf, "CIRCLE('',#{ax},");
                push_step_real(&mut self.data_buf, radius * self.scale);
                self.data_buf.push(')');
                self.finish_entity();
                (eid, seam)
            }
            CurveKind::Ellipse {
                centre,
                normal,
                x_axis,
                major_radius,
                minor_radius,
            } => {
                let ax = self.axis2(*centre, *normal, *x_axis);
                let (_, x) = ortho_frame(*normal, *x_axis);
                let seam = [
                    centre[0] + major_radius * x[0],
                    centre[1] + major_radius * x[1],
                    centre[2] + major_radius * x[2],
                ];
                let eid = self.alloc_entity_start();
                let _ = write!(self.data_buf, "ELLIPSE('',#{ax},");
                push_step_real(&mut self.data_buf, major_radius * self.scale);
                self.data_buf.push(',');
                push_step_real(&mut self.data_buf, minor_radius * self.scale);
                self.data_buf.push(')');
                self.finish_entity();
                (eid, seam)
            }
            CurveKind::Nurbs(nc) => {
                if nc.is_pcurve {
                    if let Some(host) = &curve.sp_surface {
                        if matches!(host.kind, SurfaceKind::Plane { .. }) && !nc.rational {
                            let poles_3d: Vec<[f64; 3]> = nc
                                .poles_2d
                                .iter()
                                .map(|&[u, v]| eval_surface_pt(host, u, v))
                                .collect();
                            let first = poles_3d.first().copied().unwrap_or([0.0, 0.0, 0.0]);
                            let lifted = NurbsCurve {
                                degree: nc.degree,
                                poles: poles_3d,
                                knots: nc.knots.clone(),
                                multiplicities: nc.multiplicities.clone(),
                                weights: None,
                                periodic: nc.periodic,
                                closed: nc.closed,
                                rational: false,
                                vertex_dim: 3,
                                is_pcurve: false,
                                poles_2d: Vec::new(),
                                host_surface_index: nc.host_surface_index,
                            };
                            return (self.emit_nurbs_curve(&lifted), first);
                        }
                    }
                    let u_exp = expand_knots(&nc.knots, &nc.multiplicities);
                    let (t0, t1) = if let (Some(&a), Some(&b)) = (u_exp.first(), u_exp.last()) {
                        (a, b)
                    } else {
                        (0.0, 1.0)
                    };
                    let nsamp = (2 * nc.poles_2d.len() + 1).max(17);
                    let default_plane = SurfaceData {
                        sense: crate::model::Sense::Plus,
                        kind: SurfaceKind::Plane {
                            origin: [0.0, 0.0, 0.0],
                            normal: [0.0, 0.0, 1.0],
                            x_axis: [1.0, 0.0, 0.0],
                        },
                    };
                    let host_ref = curve.sp_surface.as_ref().unwrap_or(&default_plane);
                    let mut sampled_3d = Vec::with_capacity(nsamp);
                    for k in 0..nsamp {
                        let t = t0 + (t1 - t0) * (k as f64) / ((nsamp - 1) as f64);
                        let uv = eval_bspline_dim(
                            t,
                            nc.degree,
                            &nc.knots,
                            &nc.multiplicities,
                            &nc.poles_2d,
                            nc.weights.as_deref(),
                        );
                        sampled_3d.push(eval_surface_pt(host_ref, uv[0], uv[1]));
                    }
                    let mut lifted = interpolate_cubic_bspline(&sampled_3d);
                    lifted.closed = nc.closed;
                    let first = sampled_3d[0];
                    return (self.emit_nurbs_curve(&lifted), first);
                }
                let first = nc.poles.first().copied().unwrap_or([0.0, 0.0, 0.0]);
                (self.emit_nurbs_curve(nc), first)
            }
            CurveKind::Intersection { points } => {
                if points.len() >= 2 {
                    let interp = interpolate_cubic_bspline(points);
                    return (self.emit_nurbs_curve(&interp), points[0]);
                }
                let p = self.point([0.0, 0.0, 0.0]);
                let v = self.vector([1.0, 0.0, 0.0], 1.0);
                let eid = self.alloc_entity_start();
                let _ = write!(self.data_buf, "LINE('',#{p},#{v})");
                self.finish_entity();
                (eid, [0.0, 0.0, 0.0])
            }
            CurveKind::Unsupported(_) => {
                let p = self.point([0.0, 0.0, 0.0]);
                let v = self.vector([1.0, 0.0, 0.0], 1.0);
                let eid = self.alloc_entity_start();
                let _ = write!(self.data_buf, "LINE('',#{p},#{v})");
                self.finish_entity();
                (eid, [0.0, 0.0, 0.0])
            }
        }
    }

    fn emit_surface(&mut self, surf_idx: Option<i64>, surf: &SurfaceData) -> (usize, bool) {
        if let Some(idx) = surf_idx {
            if let Some(&cached) = self.surf_cache.get(&idx) {
                return cached;
            }
        }
        let mut flip = false;
        let sid = match &surf.kind {
            SurfaceKind::Plane {
                origin,
                normal,
                x_axis,
            } => {
                let ax = self.axis2(*origin, *normal, *x_axis);
                let eid = self.alloc_entity_start();
                let _ = write!(self.data_buf, "PLANE('',#{ax})");
                self.finish_entity();
                eid
            }
            SurfaceKind::Cylinder {
                origin,
                axis,
                x_axis,
                radius,
            } => {
                let ax = self.axis2(*origin, *axis, *x_axis);
                let eid = self.alloc_entity_start();
                let _ = write!(self.data_buf, "CYLINDRICAL_SURFACE('',#{ax},");
                push_step_real(&mut self.data_buf, radius * self.scale);
                self.data_buf.push(')');
                self.finish_entity();
                eid
            }
            SurfaceKind::Cone {
                origin,
                axis,
                x_axis,
                radius,
                sin_half_angle,
                cos_half_angle,
            } => {
                let neg_a = [-axis[0], -axis[1], -axis[2]];
                let ax = self.axis2(*origin, neg_a, *x_axis);
                let ha = sin_half_angle.abs().atan2(*cos_half_angle);
                let eid = self.alloc_entity_start();
                let _ = write!(self.data_buf, "CONICAL_SURFACE('',#{ax},");
                push_step_real(&mut self.data_buf, radius * self.scale);
                self.data_buf.push(',');
                push_step_real(&mut self.data_buf, ha);
                self.data_buf.push(')');
                self.finish_entity();
                flip = true;
                eid
            }
            SurfaceKind::Sphere {
                centre,
                axis,
                x_axis,
                radius,
            } => {
                let ax = self.axis2(*centre, *axis, *x_axis);
                let eid = self.alloc_entity_start();
                let _ = write!(self.data_buf, "SPHERICAL_SURFACE('',#{ax},");
                push_step_real(&mut self.data_buf, radius * self.scale);
                self.data_buf.push(')');
                self.finish_entity();
                eid
            }
            SurfaceKind::Torus {
                centre,
                axis,
                x_axis,
                major_radius,
                minor_radius,
            } => {
                let ax = self.axis2(*centre, *axis, *x_axis);
                let eid = self.alloc_entity_start();
                let _ = write!(self.data_buf, "TOROIDAL_SURFACE('',#{ax},");
                push_step_real(&mut self.data_buf, major_radius * self.scale);
                self.data_buf.push(',');
                push_step_real(&mut self.data_buf, minor_radius * self.scale);
                self.data_buf.push(')');
                self.finish_entity();
                eid
            }
            SurfaceKind::Nurbs(ns) => self.emit_nurbs_surface(ns),
            SurfaceKind::Swept {
                direction,
                profile: Some(prof),
                ..
            } => {
                let (prof_id, _) = self.emit_curve_geom(prof);
                let vec = self.vector(*direction, 1.0);
                let eid = self.alloc_entity_start();
                let _ = write!(self.data_buf, "SURFACE_OF_LINEAR_EXTRUSION('',#{prof_id},#{vec})");
                self.finish_entity();
                eid
            }
            SurfaceKind::Spun {
                base,
                axis,
                profile: Some(prof),
                ..
            } => {
                let (prof_id, _) = self.emit_curve_geom(prof);
                let ax1 = self.axis1(*base, *axis);
                let eid = self.alloc_entity_start();
                let _ = write!(self.data_buf, "SURFACE_OF_REVOLUTION('',#{prof_id},#{ax1})");
                self.finish_entity();
                eid
            }
            SurfaceKind::Offset {
                offset,
                base_surface_index,
                base_surface: Some(base_surf),
            } => {
                let (base_id, base_flip) = self.emit_surface(*base_surface_index, base_surf);
                let eid = self.alloc_entity_start();
                let _ = write!(self.data_buf, "OFFSET_SURFACE('',#{base_id},");
                push_step_real(&mut self.data_buf, offset * self.scale);
                self.data_buf.push_str(",.F.)");
                self.finish_entity();
                flip = base_flip;
                eid
            }
            _ => {
                let ax = self.axis2([0.0, 0.0, 0.0], [0.0, 0.0, 1.0], [1.0, 0.0, 0.0]);
                let eid = self.alloc_entity_start();
                let _ = write!(self.data_buf, "PLANE('',#{ax})");
                self.finish_entity();
                eid
            }
        };

        if let Some(idx) = surf_idx {
            self.surf_cache.insert(idx, (sid, flip));
        }
        (sid, flip)
    }

    fn emit_vertex(&mut self, v_idx: i64, pt: [f64; 3]) -> usize {
        if let Some(&vid) = self.vert_cache.get(&v_idx) {
            return vid;
        }
        let p = self.point(pt);
        let eid = self.alloc_entity_start();
        let _ = write!(self.data_buf, "VERTEX_POINT('',#{p})");
        self.finish_entity();
        self.vert_cache.insert(v_idx, eid);
        eid
    }

    fn emit_edge(
        &mut self,
        edge: &Edge,
        first_coedge: &Coedge,
        model: &BrepModel,
    ) -> (usize, usize, usize, bool) {
        if let Some(&cached) = self.edge_cache.get(&edge.index) {
            return cached;
        }
        let (cid, fallback_pt) = self.emit_curve_geom(&edge.curve);
        let (sv_idx, ev_idx) = if first_coedge.sense.is_plus() {
            (first_coedge.start_vertex, first_coedge.end_vertex)
        } else {
            (first_coedge.end_vertex, first_coedge.start_vertex)
        };

        let v1_id = match sv_idx.and_then(|i| model.vertices.get(&i)) {
            Some(v) => self.emit_vertex(v.index, v.point),
            None => {
                let p = self.point(fallback_pt);
                let eid = self.alloc_entity_start();
                let _ = write!(self.data_buf, "VERTEX_POINT('',#{p})");
                self.finish_entity();
                eid
            }
        };
        let v2_id = match ev_idx.and_then(|i| model.vertices.get(&i)) {
            Some(v) => self.emit_vertex(v.index, v.point),
            None => v1_id,
        };

        let mut edge_same_sense = edge.sense.is_plus();
        if let CurveKind::Intersection { points } = &edge.curve.kind {
            if let (Some(si), Some(ei)) = (sv_idx, ev_idx) {
                if si != ei && points.len() >= 2 {
                    if let (Some(vs), Some(ve)) = (model.vertices.get(&si), model.vertices.get(&ei)) {
                        let p0 = points[0];
                        let p1 = points[points.len() - 1];
                        let d_fwd = sq_dist(p0, vs.point) + sq_dist(p1, ve.point);
                        let d_rev = sq_dist(p1, vs.point) + sq_dist(p0, ve.point);
                        edge_same_sense = d_fwd <= d_rev;
                    }
                }
            }
        }

        let eid = self.alloc_entity_start();
        let _ = write!(
            self.data_buf,
            "EDGE_CURVE('',#{v1_id},#{v2_id},#{cid},{})",
            step_bool(edge_same_sense)
        );
        self.finish_entity();
        let res = (eid, v1_id, v2_id, edge_same_sense);
        self.edge_cache.insert(edge.index, res);
        res
    }

    fn build(mut self, model: &BrepModel) -> String {
        let app_ctx = self.emit_str(match self.schema {
            StepSchema::Ap214 => {
                "APPLICATION_CONTEXT('core data for automotive mechanical design processes')"
            }
            StepSchema::Ap242 => "APPLICATION_CONTEXT('managed model based 3d engineering')",
        });
        let proto = match self.schema {
            StepSchema::Ap214 => format!(
                "APPLICATION_PROTOCOL_DEFINITION('international standard','automotive_design',2000,#{app_ctx})"
            ),
            StepSchema::Ap242 => format!(
                "APPLICATION_PROTOCOL_DEFINITION('international standard','ap242_managed_model_based_3d_engineering',2014,#{app_ctx})"
            ),
        };
        self.emit_str(&proto);
        let prod_ctx = self.emit_str(&format!("PRODUCT_CONTEXT('',#{app_ctx},'mechanical')"));
        let mut s_name = String::new();
        push_step_str(&mut s_name, &self.name);
        let prod = self.emit_str(&format!("PRODUCT({s_name},{s_name},'',(#{prod_ctx}))"));
        let pdf = self.emit_str(&format!("PRODUCT_DEFINITION_FORMATION('','',#{prod})"));
        let pdc = self.emit_str(&format!(
            "PRODUCT_DEFINITION_CONTEXT('part definition',#{app_ctx},'design')"
        ));
        let pd = self.emit_str(&format!("PRODUCT_DEFINITION('design','',#{pdf},#{pdc})"));
        let pds = self.emit_str(&format!("PRODUCT_DEFINITION_SHAPE('','',#{pd})"));

        let len_u = self.emit_str(match self.units {
            StepUnits::Mm => "(LENGTH_UNIT() NAMED_UNIT(*) SI_UNIT(.MILLI.,.METRE.))",
            StepUnits::M => "(LENGTH_UNIT() NAMED_UNIT(*) SI_UNIT($,.METRE.))",
        });
        let ang_u = self.emit_str("(NAMED_UNIT(*) PLANE_ANGLE_UNIT() SI_UNIT($,.RADIAN.))");
        let sol_u = self.emit_str("(NAMED_UNIT(*) SI_UNIT($,.STERADIAN.) SOLID_ANGLE_UNIT())");
        let unc = self.emit_str(&format!(
            "UNCERTAINTY_MEASURE_WITH_UNIT(LENGTH_MEASURE(1.E-6),#{len_u},'distance_accuracy_value','confusion accuracy')"
        ));
        let geom_ctx = self.emit_str(&format!(
            "(GEOMETRIC_REPRESENTATION_CONTEXT(3) GLOBAL_UNCERTAINTY_ASSIGNED_CONTEXT((#{unc})) GLOBAL_UNIT_ASSIGNED_CONTEXT((#{len_u},#{ang_u},#{sol_u})) REPRESENTATION_CONTEXT('Context #1','3D Context with UNIT and UNCERTAINTY'))"
        ));

        let root_ax = self.axis2([0.0, 0.0, 0.0], [0.0, 0.0, 1.0], [1.0, 0.0, 0.0]);

        let mut first_coedge_for_edge: BTreeMap<i64, &Coedge> = BTreeMap::new();
        for f in model.faces.values() {
            for lp in &f.loops {
                for ce in &lp.coedges {
                    first_coedge_for_edge.entry(ce.edge_index).or_insert(ce);
                }
            }
        }

        let mut shell_body_type: BTreeMap<Option<i64>, i64> = BTreeMap::new();
        for b in &model.bodies {
            let bt = b.body_type.unwrap_or(1);
            for &sh in &b.shell_indices {
                shell_body_type.entry(Some(sh)).or_insert(bt);
            }
        }

        let mut shell_faces: BTreeMap<Option<i64>, Vec<&Face>> = BTreeMap::new();
        for f in model.faces.values() {
            shell_faces.entry(f.shell_index).or_default().push(f);
        }

        let num_shells = shell_faces.len();
        let mut solid_items = Vec::new();
        let mut sheet_items = Vec::new();

        for (sh_idx, faces_in_shell) in shell_faces {
            let mut face_ids = Vec::new();
            let mut edge_use_count: BTreeMap<i64, usize> = BTreeMap::new();

            for f in faces_in_shell {
                let (sid, surf_flip) = self.emit_surface(f.surface_index, &f.surface);
                let outer_idx = pick_outer_loop_index(f, model);
                let mut bound_ids = Vec::new();

                for (li, lp) in f.loops.iter().enumerate() {
                    let mut oe_ids = Vec::new();
                    for ce in &lp.coedges {
                        let Some(edge) = model.edges.get(&ce.edge_index) else {
                            continue;
                        };
                        *edge_use_count.entry(edge.index).or_insert(0) += 1;
                        let ref_ce = first_coedge_for_edge[&edge.index];
                        let (ec_id, _, _, _) = self.emit_edge(edge, ref_ce, model);
                        let oe_id = self.alloc_entity_start();
                        let _ = write!(
                            self.data_buf,
                            "ORIENTED_EDGE('',*,*,#{ec_id},{})",
                            step_bool(ce.sense.is_plus())
                        );
                        self.finish_entity();
                        oe_ids.push(oe_id);
                    }
                    if oe_ids.is_empty() {
                        continue;
                    }
                    let loop_id = self.alloc_entity_start();
                    self.data_buf.push_str("EDGE_LOOP('',(");
                    push_id_list(&mut self.data_buf, &oe_ids);
                    self.data_buf.push_str("))");
                    self.finish_entity();

                    let btype = if li == outer_idx {
                        "FACE_OUTER_BOUND"
                    } else {
                        "FACE_BOUND"
                    };
                    let bid = self.alloc_entity_start();
                    let _ = write!(self.data_buf, "{btype}('',#{loop_id},.T.)");
                    self.finish_entity();
                    bound_ids.push(bid);
                }

                if bound_ids.is_empty() {
                    continue;
                }
                let face_sense = if surf_flip {
                    !f.sense.is_plus()
                } else {
                    f.sense.is_plus()
                };
                let af_id = self.alloc_entity_start();
                self.data_buf.push_str("ADVANCED_FACE('',(");
                push_id_list(&mut self.data_buf, &bound_ids);
                let _ = write!(self.data_buf, "),#{sid},{})", step_bool(face_sense));
                self.finish_entity();
                face_ids.push(af_id);
            }

            if face_ids.is_empty() {
                continue;
            }
            let btype = shell_body_type.get(&sh_idx).copied().unwrap_or(1);
            let is_closed = !face_ids.is_empty()
                && edge_use_count.values().all(|&c| c >= 2)
                && btype != 2;

            if is_closed {
                let shell = self.alloc_entity_start();
                self.data_buf.push_str("CLOSED_SHELL('',(");
                push_id_list(&mut self.data_buf, &face_ids);
                self.data_buf.push_str("))");
                self.finish_entity();

                let item_name = if num_shells > 1 {
                    format!("{}_{}", self.name, solid_items.len() + 1)
                } else {
                    self.name.clone()
                };
                let msb = self.alloc_entity_start();
                self.data_buf.push_str("MANIFOLD_SOLID_BREP(");
                push_step_str(&mut self.data_buf, &item_name);
                let _ = write!(self.data_buf, ",#{shell})");
                self.finish_entity();
                solid_items.push(msb);
            } else {
                let shell = self.alloc_entity_start();
                self.data_buf.push_str("OPEN_SHELL('',(");
                push_id_list(&mut self.data_buf, &face_ids);
                self.data_buf.push_str("))");
                self.finish_entity();

                let sbsm = self.alloc_entity_start();
                let _ = write!(self.data_buf, "SHELL_BASED_SURFACE_MODEL('',(#{shell}))");
                self.finish_entity();
                sheet_items.push(sbsm);
            }
        }

        let mut all_items = Vec::with_capacity(solid_items.len() + sheet_items.len() + 1);
        all_items.extend_from_slice(&solid_items);
        all_items.extend_from_slice(&sheet_items);
        all_items.push(root_ax);

        let rep_kind = if !solid_items.is_empty() && sheet_items.is_empty() {
            "ADVANCED_BREP_SHAPE_REPRESENTATION"
        } else if !sheet_items.is_empty() && solid_items.is_empty() {
            "MANIFOLD_SURFACE_SHAPE_REPRESENTATION"
        } else {
            "SHAPE_REPRESENTATION"
        };
        let rep = self.alloc_entity_start();
        let _ = write!(self.data_buf, "{rep_kind}('',(");
        push_id_list(&mut self.data_buf, &all_items);
        let _ = write!(self.data_buf, "),#{geom_ctx})");
        self.finish_entity();
        self.emit_str(&format!("SHAPE_DEFINITION_REPRESENTATION(#{pds},#{rep})"));

        let schema_str = match self.schema {
            StepSchema::Ap214 => "AUTOMOTIVE_DESIGN { 1 0 10303 214 1 1 1 1 }",
            StepSchema::Ap242 => "AP242_MANAGED_MODEL_BASED_3D_ENGINEERING_MIM_LF { 1 0 10303 442 1 1 4 }",
        };
        let ap_num = match self.schema {
            StepSchema::Ap214 => "214",
            StepSchema::Ap242 => "242",
        };
        let mut fname_str = String::new();
        push_step_str(&mut fname_str, &format!("{}.step", self.name));

        let mut out = String::with_capacity(self.data_buf.len() + 512);
        let _ = writeln!(out, "ISO-10303-21;");
        let _ = writeln!(out, "HEADER;");
        let _ = writeln!(out, "FILE_DESCRIPTION(('xtopen STEP AP{ap_num} export'),'2;1');");
        let _ = writeln!(
            out,
            "FILE_NAME({fname_str},'{}',('xtopen'),(''),'xtopen','','');",
            self.timestamp
        );
        let _ = writeln!(out, "FILE_SCHEMA(('{schema_str}'));");
        let _ = writeln!(out, "ENDSEC;");
        let _ = writeln!(out, "DATA;");
        out.push_str(&self.data_buf);
        let _ = writeln!(out, "ENDSEC;");
        let _ = writeln!(out, "END-ISO-10303-21;");
        out
    }
}

fn sq_dist(a: [f64; 3], b: [f64; 3]) -> f64 {
    let dx = a[0] - b[0];
    let dy = a[1] - b[1];
    let dz = a[2] - b[2];
    dx * dx + dy * dy + dz * dz
}

fn push_id_list(buf: &mut String, ids: &[usize]) {
    for (i, &id) in ids.iter().enumerate() {
        if i > 0 {
            buf.push(',');
        }
        let _ = write!(buf, "#{id}");
    }
}

fn push_usize_list(buf: &mut String, vals: &[usize]) {
    for (i, &v) in vals.iter().enumerate() {
        if i > 0 {
            buf.push(',');
        }
        let _ = write!(buf, "{v}");
    }
}

fn push_real_list(buf: &mut String, vals: &[f64]) {
    for (i, &v) in vals.iter().enumerate() {
        if i > 0 {
            buf.push(',');
        }
        push_step_real(buf, v);
    }
}

fn loop_span(lp: &Loop, model: &BrepModel) -> f64 {
    let mut pts = Vec::new();
    for ce in &lp.coedges {
        if let Some(vi) = ce.start_vertex {
            if let Some(v) = model.vertices.get(&vi) {
                pts.push(v.point);
            }
        }
        if let Some(edge) = model.edges.get(&ce.edge_index) {
            match &edge.curve.kind {
                CurveKind::Circle { radius, .. } => return *radius,
                CurveKind::Ellipse { major_radius, .. } => return *major_radius,
                _ => {}
            }
        }
    }
    if pts.is_empty() {
        return 0.0;
    }
    let mut min = pts[0];
    let mut max = pts[0];
    for &p in &pts[1..] {
        for c in 0..3 {
            if p[c] < min[c] {
                min[c] = p[c];
            }
            if p[c] > max[c] {
                max[c] = p[c];
            }
        }
    }
    (max[0] - min[0]).powi(2) + (max[1] - min[1]).powi(2) + (max[2] - min[2]).powi(2)
}

fn pick_outer_loop_index(face: &Face, model: &BrepModel) -> usize {
    if face.loops.len() <= 1 {
        return 0;
    }
    let mut best_i = 0;
    let mut best_span = -1.0;
    for (i, lp) in face.loops.iter().enumerate() {
        let sp = loop_span(lp, model);
        if sp > best_span {
            best_span = sp;
            best_i = i;
        }
    }
    best_i
}

pub fn dumps_step(model: &BrepModel, opts: &StepOptions) -> String {
    StepBuilder::new(opts).build(model)
}

pub fn convert_parsed_to_step(parsed: &ParseResult, opts: &StepOptions) -> Result<String, XtError> {
    let model = extract_brep(parsed);
    if model.faces.is_empty() {
        return Err(XtError::Unsupported(
            "Parasolid file contains no B-rep faces to import".into(),
        ));
    }
    Ok(dumps_step(&model, opts))
}
