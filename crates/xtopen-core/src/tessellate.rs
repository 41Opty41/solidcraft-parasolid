//! Pure-Rust B-rep tessellation (planar ear-clipping with holes + analytic/NURBS surfaces),
//! respecting Parasolid assembly transforms and SDL/TYSA_COLOUR appearance attributes,
//! plus binary STL and ASCII OBJ exporters.

use std::collections::{BTreeMap, BTreeSet};
use std::f64::consts::{PI, TAU};
use std::fmt::Write as _;

use crate::model::{
    transform_curve_data, transform_surface_data, Appearance, AssemblyInfo, BodyInfo, BrepModel,
    CurveData, CurveKind, Face, NurbsCurve, Sense, SurfaceData, SurfaceKind, Transform3D,
};

#[derive(Clone, Debug)]
pub struct TessMesh {
    /// Flattened xyz positions in millimeters (`3 * n_render_verts`).
    pub render_positions: Vec<f32>,
    /// Flattened vertex normals (`3 * n_render_verts`).
    pub render_normals: Vec<f32>,
    /// Flattened RGBA/RGB colors (`3 * n_render_verts`, 0..1).
    pub render_colors: Vec<f32>,
    /// Triangle index list into `render_positions` (`3 * n_tris`).
    pub render_indices: Vec<u32>,
    /// Edge wireframe line segments (`x0,y0,z0, x1,y1,z1`, in mm).
    pub edge_lines: Vec<f32>,
    /// Welded f64 positions in millimeters for exact mass/inertia/watertightness analysis.
    pub mass_positions_mm: Vec<[f64; 3]>,
    /// Triangle index list into `mass_positions_mm`.
    pub mass_triangles: Vec<[u32; 3]>,
}

fn sub3(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn dot3(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross3(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn norm3(v: [f64; 3]) -> [f64; 3] {
    let n = dot3(v, v).sqrt();
    if n < 1e-15 {
        [0.0, 0.0, 1.0]
    } else {
        [v[0] / n, v[1] / n, v[2] / n]
    }
}

fn ortho_frame(z_raw: [f64; 3], x_raw: [f64; 3]) -> ([f64; 3], [f64; 3], [f64; 3]) {
    let z = norm3(z_raw);
    let d = dot3(x_raw, z);
    let mut xp = [x_raw[0] - d * z[0], x_raw[1] - d * z[1], x_raw[2] - d * z[2]];
    if dot3(xp, xp).sqrt() < 1e-12 {
        let cand = if z[0].abs() < 0.9 {
            [1.0, 0.0, 0.0]
        } else {
            [0.0, 1.0, 0.0]
        };
        xp = cross3(z, cand);
    }
    let x = norm3(xp);
    let y = cross3(z, x);
    (x, y, z)
}

fn expand_knots(knots: &[f64], mults: &[usize]) -> Vec<f64> {
    let mut out = Vec::with_capacity(mults.iter().sum());
    for (&k, &m) in knots.iter().zip(mults.iter()) {
        out.extend(std::iter::repeat_n(k, m));
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

fn eval_curve_pt(curve: &CurveData, t: f64) -> [f64; 3] {
    match &curve.kind {
        CurveKind::Line { origin, dir } => [
            origin[0] + t * dir[0],
            origin[1] + t * dir[1],
            origin[2] + t * dir[2],
        ],
        CurveKind::Circle {
            centre,
            normal,
            x_axis,
            radius,
        } => {
            let (x, y, _) = ortho_frame(*normal, *x_axis);
            let (s, c) = t.sin_cos();
            [
                centre[0] + radius * (c * x[0] + s * y[0]),
                centre[1] + radius * (c * x[1] + s * y[1]),
                centre[2] + radius * (c * x[2] + s * y[2]),
            ]
        }
        CurveKind::Ellipse {
            centre,
            normal,
            x_axis,
            major_radius,
            minor_radius,
        } => {
            let (x, y, _) = ortho_frame(*normal, *x_axis);
            let (s, c) = t.sin_cos();
            [
                centre[0] + major_radius * c * x[0] + minor_radius * s * y[0],
                centre[1] + major_radius * c * x[1] + minor_radius * s * y[1],
                centre[2] + major_radius * c * x[2] + minor_radius * s * y[2],
            ]
        }
        CurveKind::Nurbs(nc) => eval_nurbs_curve_pt(nc, curve.sp_surface.as_ref(), t),
        _ => [0.0, 0.0, 0.0],
    }
}

fn eval_nurbs_curve_pt(nc: &NurbsCurve, sp_surface: Option<&SurfaceData>, t: f64) -> [f64; 3] {
    if nc.is_pcurve {
        let uv = eval_bspline_dim(
            t,
            nc.degree,
            &nc.knots,
            &nc.multiplicities,
            &nc.poles_2d,
            nc.weights.as_deref(),
        );
        if let Some(s) = sp_surface {
            return eval_surface_pt(s, uv[0], uv[1]);
        }
        return [uv[0], uv[1], 0.0];
    }
    eval_bspline_dim(
        t,
        nc.degree,
        &nc.knots,
        &nc.multiplicities,
        &nc.poles,
        nc.weights.as_deref(),
    )
}

fn eval_surface_pt(surf: &SurfaceData, u: f64, v: f64) -> [f64; 3] {
    match &surf.kind {
        SurfaceKind::Plane {
            origin,
            normal,
            x_axis,
        } => {
            let (x, y, _) = ortho_frame(*normal, *x_axis);
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
            let (x, y, z) = ortho_frame(*axis, *x_axis);
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
            let (x, y, z) = ortho_frame(*axis, *x_axis);
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
            let (x, y, z) = ortho_frame(*axis, *x_axis);
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
            let (x, y, z) = ortho_frame(*axis, *x_axis);
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
                return [0.0; 3];
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
        SurfaceKind::Swept {
            direction,
            scale: _,
            profile: Some(prof),
            ..
        } => {
            let p = eval_curve_pt(prof, u);
            [
                p[0] + v * direction[0],
                p[1] + v * direction[1],
                p[2] + v * direction[2],
            ]
        }
        SurfaceKind::Spun {
            base,
            axis,
            profile: Some(prof),
            ..
        } => {
            let p = eval_curve_pt(prof, u);
            let z = norm3(*axis);
            let d = sub3(p, *base);
            let (s, c) = v.sin_cos();
            let par = dot3(d, z);
            let k_cross_d = cross3(z, d);
            [
                base[0] + d[0] * c + k_cross_d[0] * s + z[0] * par * (1.0 - c),
                base[1] + d[1] * c + k_cross_d[1] * s + z[1] * par * (1.0 - c),
                base[2] + d[2] * c + k_cross_d[2] * s + z[2] * par * (1.0 - c),
            ]
        }
        SurfaceKind::Offset {
            offset,
            base_surface: Some(base),
            ..
        } => {
            let p = eval_surface_pt(base, u, v);
            let n = eval_surface_normal(base, u, v, Sense::Plus);
            [p[0] + offset * n[0], p[1] + offset * n[1], p[2] + offset * n[2]]
        }
        _ => [0.0; 3],
    }
}

fn eval_surface_normal(surf: &SurfaceData, u: f64, v: f64, sense: Sense) -> [f64; 3] {
    let mut n = match &surf.kind {
        SurfaceKind::Plane { normal, .. } => norm3(*normal),
        SurfaceKind::Cylinder {
            axis, x_axis, ..
        } => {
            let (x, y, _) = ortho_frame(*axis, *x_axis);
            let (su, cu) = u.sin_cos();
            [cu * x[0] + su * y[0], cu * x[1] + su * y[1], cu * x[2] + su * y[2]]
        }
        SurfaceKind::Cone {
            axis,
            x_axis,
            sin_half_angle,
            cos_half_angle,
            ..
        } => {
            let (x, y, z) = ortho_frame(*axis, *x_axis);
            let (su, cu) = u.sin_cos();
            let r_dir = [cu * x[0] + su * y[0], cu * x[1] + su * y[1], cu * x[2] + su * y[2]];
            norm3([
                cos_half_angle * r_dir[0] + sin_half_angle * z[0],
                cos_half_angle * r_dir[1] + sin_half_angle * z[1],
                cos_half_angle * r_dir[2] + sin_half_angle * z[2],
            ])
        }
        SurfaceKind::Sphere {
            centre,
            axis,
            x_axis,
            radius,
        } => {
            let _ = (centre, radius);
            let (x, y, z) = ortho_frame(*axis, *x_axis);
            let (su, cu) = u.sin_cos();
            let (sv, cv) = v.sin_cos();
            [
                cv * (cu * x[0] + su * y[0]) + sv * z[0],
                cv * (cu * x[1] + su * y[1]) + sv * z[1],
                cv * (cu * x[2] + su * y[2]) + sv * z[2],
            ]
        }
        SurfaceKind::Torus {
            axis, x_axis, ..
        } => {
            let (x, y, z) = ortho_frame(*axis, *x_axis);
            let (su, cu) = u.sin_cos();
            let (sv, cv) = v.sin_cos();
            let r_dir = [cu * x[0] + su * y[0], cu * x[1] + su * y[1], cu * x[2] + su * y[2]];
            [
                cv * r_dir[0] + sv * z[0],
                cv * r_dir[1] + sv * z[1],
                cv * r_dir[2] + sv * z[2],
            ]
        }
        SurfaceKind::Offset {
            base_surface: Some(base),
            ..
        } => eval_surface_normal(base, u, v, Sense::Plus),
        _ => {
            let eps = 1e-4;
            let p_u1 = eval_surface_pt(surf, u + eps, v);
            let p_u0 = eval_surface_pt(surf, u - eps, v);
            let p_v1 = eval_surface_pt(surf, u, v + eps);
            let p_v0 = eval_surface_pt(surf, u, v - eps);
            norm3(cross3(sub3(p_u1, p_u0), sub3(p_v1, p_v0)))
        }
    };
    if !sense.is_plus() {
        n = [-n[0], -n[1], -n[2]];
    }
    n
}

fn sample_edge_points(
    curve: &CurveData,
    p_start: [f64; 3],
    p_end: [f64; 3],
    fwd_dir: bool,
    n_arc_full: usize,
) -> Vec<[f64; 3]> {
    let mut pts = Vec::new();
    match &curve.kind {
        CurveKind::Line { .. } => {
            pts.push(p_start);
            pts.push(p_end);
        }
        CurveKind::Circle {
            centre,
            normal,
            x_axis,
            radius,
        } => {
            let (x, y, _) = ortho_frame(*normal, *x_axis);
            let ds = sub3(p_start, *centre);
            let de = sub3(p_end, *centre);
            let a0 = dot3(ds, y).atan2(dot3(ds, x));
            let a1 = dot3(de, y).atan2(dot3(de, x));
            let mut sweep = if fwd_dir { a1 - a0 } else { a0 - a1 };
            if dot3(sub3(p_start, p_end), sub3(p_start, p_end)) < 1e-18 {
                sweep = TAU;
            } else {
                while sweep <= 1e-9 {
                    sweep += TAU;
                }
                while sweep > TAU + 1e-9 {
                    sweep -= TAU;
                }
            }
            let n_seg = ((sweep / TAU * (n_arc_full as f64)).ceil() as usize).max(6);
            pts.push(p_start);
            for k in 1..n_seg {
                let frac = (k as f64) / (n_seg as f64);
                let ang = if fwd_dir {
                    a0 + sweep * frac
                } else {
                    a0 - sweep * frac
                };
                let (sa, ca) = ang.sin_cos();
                pts.push([
                    centre[0] + radius * (ca * x[0] + sa * y[0]),
                    centre[1] + radius * (ca * x[1] + sa * y[1]),
                    centre[2] + radius * (ca * x[2] + sa * y[2]),
                ]);
            }
            pts.push(p_end);
        }
        CurveKind::Ellipse {
            centre,
            normal,
            x_axis,
            major_radius,
            minor_radius,
        } => {
            let (x, y, _) = ortho_frame(*normal, *x_axis);
            let ds = sub3(p_start, *centre);
            let de = sub3(p_end, *centre);
            let a0 = (dot3(ds, y) / minor_radius.max(1e-12))
                .atan2(dot3(ds, x) / major_radius.max(1e-12));
            let a1 = (dot3(de, y) / minor_radius.max(1e-12))
                .atan2(dot3(de, x) / major_radius.max(1e-12));
            let mut sweep = if fwd_dir { a1 - a0 } else { a0 - a1 };
            if dot3(sub3(p_start, p_end), sub3(p_start, p_end)) < 1e-18 {
                sweep = TAU;
            } else {
                while sweep <= 1e-9 {
                    sweep += TAU;
                }
                while sweep > TAU + 1e-9 {
                    sweep -= TAU;
                }
            }
            let n_seg = ((sweep / TAU * (n_arc_full as f64)).ceil() as usize).max(6);
            pts.push(p_start);
            for k in 1..n_seg {
                let frac = (k as f64) / (n_seg as f64);
                let ang = if fwd_dir {
                    a0 + sweep * frac
                } else {
                    a0 - sweep * frac
                };
                let (sa, ca) = ang.sin_cos();
                pts.push([
                    centre[0] + major_radius * ca * x[0] + minor_radius * sa * y[0],
                    centre[1] + major_radius * ca * x[1] + minor_radius * sa * y[1],
                    centre[2] + major_radius * ca * x[2] + minor_radius * sa * y[2],
                ]);
            }
            pts.push(p_end);
        }
        CurveKind::Nurbs(nc) => {
            let u = expand_knots(&nc.knots, &nc.multiplicities);
            let (t0, t1) = if let (Some(&a), Some(&b)) = (u.first(), u.last()) {
                (a, b)
            } else {
                (0.0, 1.0)
            };
            let n_seg = 24usize;
            let p0 = eval_nurbs_curve_pt(nc, curve.sp_surface.as_ref(), t0);
            let d_fwd = dot3(sub3(p0, p_start), sub3(p0, p_start));
            let p1 = eval_nurbs_curve_pt(nc, curve.sp_surface.as_ref(), t1);
            let d_rev = dot3(sub3(p1, p_start), sub3(p1, p_start));
            let forward_param = d_fwd <= d_rev;

            pts.push(p_start);
            for k in 1..n_seg {
                let frac = (k as f64) / (n_seg as f64);
                let t = if forward_param {
                    t0 + (t1 - t0) * frac
                } else {
                    t1 - (t1 - t0) * frac
                };
                pts.push(eval_nurbs_curve_pt(nc, curve.sp_surface.as_ref(), t));
            }
            pts.push(p_end);
        }
        CurveKind::Intersection { points } => {
            if points.len() >= 2 {
                let d_fwd = dot3(sub3(points[0], p_start), sub3(points[0], p_start));
                let d_rev = dot3(sub3(*points.last().unwrap(), p_start), sub3(*points.last().unwrap(), p_start));
                if d_fwd <= d_rev {
                    pts.extend_from_slice(points);
                } else {
                    pts.extend(points.iter().rev().copied());
                }
                *pts.first_mut().unwrap() = p_start;
                *pts.last_mut().unwrap() = p_end;
            } else {
                pts.push(p_start);
                pts.push(p_end);
            }
        }
        CurveKind::Unsupported(_) => {
            pts.push(p_start);
            pts.push(p_end);
        }
    }
    pts
}

fn area2_signed(pts: &[[f64; 2]]) -> f64 {
    let n = pts.len();
    let mut s = 0.0;
    for i in 0..n {
        let j = (i + 1) % n;
        s += pts[i][0] * pts[j][1] - pts[j][0] * pts[i][1];
    }
    0.5 * s
}

fn cross2(a: [f64; 2], b: [f64; 2], c: [f64; 2]) -> f64 {
    (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])
}

fn point_in_tri2(p: [f64; 2], a: [f64; 2], b: [f64; 2], c: [f64; 2]) -> bool {
    let c1 = cross2(a, b, p);
    let c2 = cross2(b, c, p);
    let c3 = cross2(c, a, p);
    c1 >= -1e-12 && c2 >= -1e-12 && c3 >= -1e-12
}

/// Bridge inner hole loops into an outer CCW loop and triangulate by ear clipping.
/// Input `loops_xy` has `outer` first (made CCW) followed by `holes` (made CW).
fn triangulate_2d_with_holes(loops_xy: Vec<Vec<[f64; 2]>>) -> (Vec<[f64; 2]>, Vec<[usize; 3]>) {
    let Some(outer_raw) = loops_xy.first() else {
        return (Vec::new(), Vec::new());
    };
    if outer_raw.len() < 3 {
        return (Vec::new(), Vec::new());
    }

    let mut all_pts: Vec<[f64; 2]> = Vec::new();
    let mut ring: Vec<usize> = Vec::with_capacity(outer_raw.len() + 32);

    if area2_signed(outer_raw) >= 0.0 {
        for &p in outer_raw {
            ring.push(all_pts.len());
            all_pts.push(p);
        }
    } else {
        for &p in outer_raw.iter().rev() {
            ring.push(all_pts.len());
            all_pts.push(p);
        }
    }

    for hole_raw in &loops_xy[1..] {
        if hole_raw.len() < 3 {
            continue;
        }
        let h_start = all_pts.len();
        let nh = hole_raw.len();
        if area2_signed(hole_raw) >= 0.0 {
            for &p in hole_raw.iter().rev() {
                all_pts.push(p);
            }
        } else {
            for &p in hole_raw {
                all_pts.push(p);
            }
        }

        let mut max_i = 0usize;
        for i in 1..nh {
            if all_pts[h_start + i][0] > all_pts[h_start + max_i][0] {
                max_i = i;
            }
        }
        let m_pt = all_pts[h_start + max_i];

        // Ray cast +X from m_pt
        let mut best_hx = f64::INFINITY;
        let mut best_edge = 0usize;
        let nr = ring.len();
        for i in 0..nr {
            let pa = all_pts[ring[i]];
            let pb = all_pts[ring[(i + 1) % nr]];
            if (pa[1] <= m_pt[1] && pb[1] > m_pt[1]) || (pb[1] <= m_pt[1] && pa[1] > m_pt[1]) {
                let t = (m_pt[1] - pa[1]) / (pb[1] - pa[1]);
                let hx = pa[0] + t * (pb[0] - pa[0]);
                if hx >= m_pt[0] - 1e-12 && hx < best_hx {
                    best_hx = hx;
                    best_edge = i;
                }
            }
        }
        let va = ring[best_edge];
        let vb = ring[(best_edge + 1) % nr];
        let mut bridge_pos = if all_pts[va][0] >= all_pts[vb][0] {
            best_edge
        } else {
            (best_edge + 1) % nr
        };
        let p_v = all_pts[ring[bridge_pos]];
        let mut best_dist2 = (p_v[0] - m_pt[0]).powi(2) + (p_v[1] - m_pt[1]).powi(2);

        for (i, &vid) in ring.iter().enumerate() {
            if i == bridge_pos {
                continue;
            }
            let vp = all_pts[vid];
            if vp[0] >= m_pt[0] && point_in_tri2(vp, m_pt, [best_hx, m_pt[1]], p_v) {
                let d2 = (vp[0] - m_pt[0]).powi(2) + (vp[1] - m_pt[1]).powi(2);
                if d2 < best_dist2 {
                    best_dist2 = d2;
                    bridge_pos = i;
                }
            }
        }

        let mut splice = Vec::with_capacity(nh + 2);
        for k in 0..nh {
            splice.push(h_start + ((max_i + k) % nh));
        }
        splice.push(h_start + max_i);
        splice.push(ring[bridge_pos]);
        ring.splice(bridge_pos + 1..bridge_pos + 1, splice);
    }

    let mut tris = Vec::new();
    let mut guard = ring.len() * ring.len() + 32;
    let mut pos = 0usize;
    while ring.len() >= 3 && guard > 0 {
        guard -= 1;
        let n = ring.len();
        let mut clipped = false;
        for step in 0..n {
            let idx = (pos + step) % n;
            let prev = (idx + n - 1) % n;
            let next = (idx + 1) % n;
            let a = ring[prev];
            let b = ring[idx];
            let c = ring[next];
            if cross2(all_pts[a], all_pts[b], all_pts[c]) <= 1e-12 {
                continue;
            }
            let mut contains = false;
            for (j, &q) in ring.iter().enumerate() {
                if j == prev || j == idx || j == next || q == a || q == b || q == c {
                    continue;
                }
                if point_in_tri2(all_pts[q], all_pts[a], all_pts[b], all_pts[c]) {
                    contains = true;
                    break;
                }
            }
            if !contains {
                tris.push([a, b, c]);
                ring.remove(idx);
                if !ring.is_empty() {
                    pos = idx % ring.len();
                }
                clipped = true;
                break;
            }
        }
        if !clipped {
            break;
        }
    }

    (all_pts, tris)
}

fn inverse_uv_hint(surf: &SurfaceData, p: [f64; 3], prev_u: Option<f64>) -> [f64; 2] {
    let wrap_angle = |raw: f64, prev: Option<f64>| -> f64 {
        let mut a = raw;
        if let Some(pu) = prev {
            while a - pu > PI {
                a -= TAU;
            }
            while a - pu < -PI {
                a += TAU;
            }
        } else if a < 0.0 {
            a += TAU;
        }
        a
    };
    match &surf.kind {
        SurfaceKind::Plane {
            origin,
            normal,
            x_axis,
        } => {
            let (x, y, _) = ortho_frame(*normal, *x_axis);
            let d = sub3(p, *origin);
            [dot3(d, x), dot3(d, y)]
        }
        SurfaceKind::Cylinder {
            origin,
            axis,
            x_axis,
            ..
        } => {
            let (x, y, z) = ortho_frame(*axis, *x_axis);
            let d = sub3(p, *origin);
            let u = wrap_angle(dot3(d, y).atan2(dot3(d, x)), prev_u);
            let v = dot3(d, z);
            [u, v]
        }
        SurfaceKind::Cone {
            origin,
            axis,
            x_axis,
            ..
        } => {
            let (x, y, z) = ortho_frame(*axis, *x_axis);
            let d = sub3(p, *origin);
            let u = wrap_angle(dot3(d, y).atan2(dot3(d, x)), prev_u);
            let v = -dot3(d, z);
            [u, v]
        }
        SurfaceKind::Sphere {
            centre,
            axis,
            x_axis,
            radius,
        } => {
            let (x, y, z) = ortho_frame(*axis, *x_axis);
            let d = sub3(p, *centre);
            let u = wrap_angle(dot3(d, y).atan2(dot3(d, x)), prev_u);
            let sv = (dot3(d, z) / radius.max(1e-12)).clamp(-1.0, 1.0);
            [u, sv.asin()]
        }
        SurfaceKind::Torus {
            centre,
            axis,
            x_axis,
            major_radius,
            ..
        } => {
            let (x, y, z) = ortho_frame(*axis, *x_axis);
            let d = sub3(p, *centre);
            let dx = dot3(d, x);
            let dy = dot3(d, y);
            let dz = dot3(d, z);
            let u = wrap_angle(dy.atan2(dx), prev_u);
            let radial = (dx * dx + dy * dy).sqrt() - major_radius;
            let v = dz.atan2(radial);
            [u, v]
        }
        _ => [0.0, 0.0],
    }
}

fn tessellate_analytic_or_nurbs_face<F>(
    face: &Face,
    loops_3d: &[Vec<[f64; 3]>],
    add_tri: &mut F,
) where
    F: FnMut([f64; 3], [f64; 3], [f64; 3], [f64; 3], [f64; 3], [f64; 3]),
{
    // Planar faces: project onto plane and ear-clip directly with exact boundary points
    if let SurfaceKind::Plane {
        origin,
        normal,
        x_axis,
    } = &face.surface.kind
    {
        let (x, y, z) = ortho_frame(*normal, *x_axis);
        let n_out = if face.sense.is_plus() {
            z
        } else {
            [-z[0], -z[1], -z[2]]
        };
        let mut loops_xy: Vec<Vec<[f64; 2]>> = loops_3d
            .iter()
            .filter(|l| l.len() >= 3)
            .map(|l| {
                l.iter()
                    .map(|&p| {
                        let d = sub3(p, *origin);
                        [dot3(d, x), dot3(d, y)]
                    })
                    .collect()
            })
            .collect();
        if loops_xy.len() > 1 {
            // Put largest absolute signed-area loop first as outer boundary
            let mut best = 0usize;
            let mut best_a = -1.0;
            for (i, lp) in loops_xy.iter().enumerate() {
                let a = area2_signed(lp).abs();
                if a > best_a {
                    best_a = a;
                    best = i;
                }
            }
            loops_xy.swap(0, best);
        }
        let (pts2, tris) = triangulate_2d_with_holes(loops_xy);
        let to3 = |q: [f64; 2]| -> [f64; 3] {
            [
                origin[0] + q[0] * x[0] + q[1] * y[0],
                origin[1] + q[0] * x[1] + q[1] * y[1],
                origin[2] + q[0] * x[2] + q[1] * y[2],
            ]
        };
        for [a, b, c] in tris {
            let pa = to3(pts2[a]);
            let pb = to3(pts2[b]);
            let pc = to3(pts2[c]);
            let tn = cross3(sub3(pb, pa), sub3(pc, pa));
            if dot3(tn, n_out) >= 0.0 {
                add_tri(pa, pb, pc, n_out, n_out, n_out);
            } else {
                add_tri(pa, pc, pb, n_out, n_out, n_out);
            }
        }
        return;
    }

    // Sphere hemisphere bounded by 1 equator-like loop: use stereographic/polar disk (r, theta) -> (r cos u, r sin u)
    if let SurfaceKind::Sphere {
        centre,
        axis,
        x_axis,
        radius,
    } = &face.surface.kind
    {
        if loops_3d.len() == 1 && loops_3d[0].len() >= 6 {
            let lp = &loops_3d[0];
            let (_, _, z) = ortho_frame(*axis, *x_axis);
            // Compute centroid height relative to equator normal to detect upper vs lower hemisphere
            let n_pts = lp.len();
            let mut signed_z_turn = 0.0;
            let mut prev_u = None;
            let mut uv_pts = Vec::with_capacity(n_pts);
            let mut v_avg = 0.0;
            for &p in lp {
                let uv = inverse_uv_hint(&face.surface, p, prev_u);
                prev_u = Some(uv[0]);
                v_avg += uv[1];
                uv_pts.push((uv, p));
            }
            v_avg /= n_pts as f64;
            for i in 0..n_pts {
                let p0 = sub3(lp[i], *centre);
                let p1 = sub3(lp[(i + 1) % n_pts], *centre);
                signed_z_turn += dot3(cross3(p0, p1), z);
            }
            let is_upper = if face.sense.is_plus() {
                signed_z_turn >= 0.0
            } else {
                signed_z_turn < 0.0
            };
            let v_pole = if is_upper { 0.5 * PI } else { -0.5 * PI };
            let sign_r = if is_upper { 1.0 } else { -1.0 };
            let v_band = (v_pole - v_avg).abs().max(0.05);

            // Build concentric latitude rings between v_avg (~boundary) and pole, snapping exact boundary points on ring 0
            let n_rings = 8usize;
            let m = lp.len();
            let mut ring_pts: Vec<Vec<([f64; 3], [f64; 3])>> = Vec::with_capacity(n_rings);
            let r0: Vec<([f64; 3], [f64; 3])> = uv_pts
                .iter()
                .map(|&(uv, p)| (p, eval_surface_normal(&face.surface, uv[0], uv[1], face.sense)))
                .collect();
            ring_pts.push(r0);
            for ri in 1..n_rings {
                let frac = (ri as f64) / (n_rings as f64);
                let mut row = Vec::with_capacity(m);
                for &(uv, _) in &uv_pts {
                    let vv = uv[1] + sign_r * frac * (v_band - 0.0) * (1.0 - 0.0);
                    let clamped_v = if is_upper {
                        vv.min(0.5 * PI - 1e-4)
                    } else {
                        vv.max(-0.5 * PI + 1e-4)
                    };
                    let pt = eval_surface_pt(&face.surface, uv[0], clamped_v);
                    let nrm = eval_surface_normal(&face.surface, uv[0], clamped_v, face.sense);
                    row.push((pt, nrm));
                }
                ring_pts.push(row);
            }
            let pole_pt = [
                centre[0] + sign_r * radius * z[0],
                centre[1] + sign_r * radius * z[1],
                centre[2] + sign_r * radius * z[2],
            ];
            let pole_n = eval_surface_normal(&face.surface, 0.0, v_pole, face.sense);

            let mut emit_oriented = |pa: [f64; 3], pb: [f64; 3], pc: [f64; 3], na: [f64; 3], nb: [f64; 3], nc: [f64; 3]| {
                let tn = cross3(sub3(pb, pa), sub3(pc, pa));
                let nav = [na[0] + nb[0] + nc[0], na[1] + nb[1] + nc[1], na[2] + nb[2] + nc[2]];
                if dot3(tn, nav) >= 0.0 {
                    add_tri(pa, pb, pc, na, nb, nc);
                } else {
                    add_tri(pa, pc, pb, na, nc, nb);
                }
            };

            for ri in 0..(n_rings - 1) {
                for k in 0..m {
                    let kn = (k + 1) % m;
                    let (a, na) = ring_pts[ri][k];
                    let (b, nb) = ring_pts[ri][kn];
                    let (c, nc) = ring_pts[ri + 1][kn];
                    let (d, nd) = ring_pts[ri + 1][k];
                    emit_oriented(a, b, c, na, nb, nc);
                    emit_oriented(a, c, d, na, nc, nd);
                }
            }
            let last = &ring_pts[n_rings - 1];
            for k in 0..m {
                let kn = (k + 1) % m;
                emit_oriented(last[k].0, last[kn].0, pole_pt, last[k].1, last[kn].1, pole_n);
            }
            return;
        }
    }

    // NURBS / analytic rectangular or closed-loop faces: evaluate in UV parameter space
    if let SurfaceKind::Nurbs(ns) = &face.surface.kind {
        let u_exp = expand_knots(&ns.u_knots, &ns.u_mults);
        let v_exp = expand_knots(&ns.v_knots, &ns.v_mults);
        if let (Some(&u0), Some(&u1), Some(&v0), Some(&v1)) =
            (u_exp.first(), u_exp.last(), v_exp.first(), v_exp.last())
        {
            let nu = 16usize;
            let nv = 16usize;
            let mut grid_p = Vec::with_capacity((nu + 1) * (nv + 1));
            let mut grid_n = Vec::with_capacity((nu + 1) * (nv + 1));
            for iu in 0..=nu {
                let u = u0 + (u1 - u0) * (iu as f64) / (nu as f64);
                for iv in 0..=nv {
                    let v = v0 + (v1 - v0) * (iv as f64) / (nv as f64);
                    grid_p.push(eval_surface_pt(&face.surface, u, v));
                    grid_n.push(eval_surface_normal(&face.surface, u, v, face.sense));
                }
            }
            for iu in 0..nu {
                for iv in 0..nv {
                    let i00 = iu * (nv + 1) + iv;
                    let i10 = (iu + 1) * (nv + 1) + iv;
                    let i11 = (iu + 1) * (nv + 1) + iv + 1;
                    let i01 = iu * (nv + 1) + iv + 1;
                    let mut emit = |a: usize, b: usize, c: usize| {
                        let pa = grid_p[a];
                        let pb = grid_p[b];
                        let pc = grid_p[c];
                        let na = grid_n[a];
                        let nb = grid_n[b];
                        let nc = grid_n[c];
                        let nav = [na[0] + nb[0] + nc[0], na[1] + nb[1] + nc[1], na[2] + nb[2] + nc[2]];
                        if dot3(cross3(sub3(pb, pa), sub3(pc, pa)), nav) >= 0.0 {
                            add_tri(pa, pb, pc, na, nb, nc);
                        } else {
                            add_tri(pa, pc, pb, na, nc, nb);
                        }
                    };
                    emit(i00, i10, i11);
                    emit(i00, i11, i01);
                }
            }
            return;
        }
    }

    // Cylinder / Cone / Torus / Swept / Spun / Offset faces bounded by loops:
    // Use boundary loop UV points!
    if loops_3d.is_empty() || loops_3d[0].len() < 3 {
        return;
    }

    // Special case: cylinder or cone face bounded by two closed circular loops (seam-free sleeve)
    if loops_3d.len() == 2
        && matches!(
            face.surface.kind,
            SurfaceKind::Cylinder { .. } | SurfaceKind::Cone { .. }
        )
    {
        let l0 = &loops_3d[0];
        let l1 = &loops_3d[1];
        if l0.len() == l1.len() && l0.len() >= 6 {
            let mut uvs0 = Vec::with_capacity(l0.len());
            let mut uvs1 = Vec::with_capacity(l1.len());
            for &p in l0 {
                uvs0.push(inverse_uv_hint(&face.surface, p, None));
            }
            for &p in l1 {
                uvs1.push(inverse_uv_hint(&face.surface, p, None));
            }
            // Match starting index on l1 closest in u to uvs0[0]
            let u0 = uvs0[0][0];
            let mut best_off = 0usize;
            let mut best_du = f64::INFINITY;
            for (j, &[uj, _]) in uvs1.iter().enumerate() {
                let mut d = (uj - u0).abs();
                if d > PI {
                    d = TAU - d;
                }
                if d < best_du {
                    best_du = d;
                    best_off = j;
                }
            }
            let dir1: isize = if uvs1[(best_off + 1) % l1.len()][0] - uvs1[best_off][0] >= 0.0 {
                if uvs0[1][0] - uvs0[0][0] >= 0.0 { 1 } else { -1 }
            } else if uvs0[1][0] - uvs0[0][0] >= 0.0 {
                -1
            } else {
                1
            };
            let m = l0.len();
            for k in 0..m {
                let kn = (k + 1) % m;
                let j0 = ((best_off as isize + dir1 * (k as isize)).rem_euclid(m as isize)) as usize;
                let j1 =
                    ((best_off as isize + dir1 * (kn as isize)).rem_euclid(m as isize)) as usize;
                let pa = l0[k];
                let pb = l0[kn];
                let pc = l1[j1];
                let pd = l1[j0];
                let na = eval_surface_normal(&face.surface, uvs0[k][0], uvs0[k][1], face.sense);
                let nb = eval_surface_normal(&face.surface, uvs0[kn][0], uvs0[kn][1], face.sense);
                let nc = eval_surface_normal(&face.surface, uvs1[j1][0], uvs1[j1][1], face.sense);
                let nd = eval_surface_normal(&face.surface, uvs1[j0][0], uvs1[j0][1], face.sense);
                let nav = [na[0] + nb[0] + nc[0], na[1] + nb[1] + nc[1], na[2] + nb[2] + nc[2]];
                if dot3(cross3(sub3(pb, pa), sub3(pc, pa)), nav) >= 0.0 {
                    add_tri(pa, pb, pc, na, nb, nc);
                    add_tri(pa, pc, pd, na, nc, nd);
                } else {
                    add_tri(pa, pc, pb, na, nc, nb);
                    add_tri(pa, pd, pc, na, nd, nc);
                }
            }
            return;
        }
    }

    // General curved patch (half-cylinder, half-cone, trimmed cylinder/cone, torus, swept, spun, offset):
    // Unwrap boundary loop to continuous (u, v) coordinates with exact 3D boundary points.
    let lp = &loops_3d[0];
    let mut prev_u = None;
    let mut uvs: Vec<[f64; 2]> = Vec::with_capacity(lp.len());
    let mut exact_pts: Vec<[f64; 3]> = Vec::with_capacity(lp.len());
    for &p in lp {
        let uv = inverse_uv_hint(&face.surface, p, prev_u);
        prev_u = Some(uv[0]);
        uvs.push(uv);
        exact_pts.push(p);
    }

    // Detect clean UV quad-strip topology (e.g., cylinder/cone barrel half: top & bottom chains at matching u's):
    // Split boundary loop into horizontal segments (where |dv| is small / du dominates) grouped into bottom (lower v) and top (upper v) chains.
    if uvs.len() >= 6 {
        let (mut u_min, mut u_max) = (uvs[0][0], uvs[0][0]);
        let (mut v_min, mut v_max) = (uvs[0][1], uvs[0][1]);
        for &[u, v] in &uvs {
            u_min = u_min.min(u);
            u_max = u_max.max(u);
            v_min = v_min.min(v);
            v_max = v_max.max(v);
        }
        let du_span = (u_max - u_min).max(1e-9);
        let dv_span = (v_max - v_min).max(1e-9);
        let tol_v = dv_span * 1e-3;
        let mut bot: Vec<(f64, usize)> = Vec::new();
        let mut top: Vec<(f64, usize)> = Vec::new();
        for (i, &[u, v]) in uvs.iter().enumerate() {
            if (v - v_min).abs() < tol_v {
                bot.push((u, i));
            } else if (v - v_max).abs() < tol_v {
                top.push((u, i));
            }
        }
        // Check whether all boundary vertices sit on bottom or top chains with vertical seams connecting them
        if bot.len() + top.len() == uvs.len() && bot.len() >= 3 && top.len() >= 3 {
            bot.sort_by(|a, b| a.0.total_cmp(&b.0));
            top.sort_by(|a, b| a.0.total_cmp(&b.0));
            if bot.len() == top.len() {
                for k in 0..(bot.len() - 1) {
                    let a = bot[k].1;
                    let b = bot[k + 1].1;
                    let c = top[k + 1].1;
                    let d = top[k].1;
                    let pa = exact_pts[a];
                    let pb = exact_pts[b];
                    let pc = exact_pts[c];
                    let pd = exact_pts[d];
                    let na = eval_surface_normal(&face.surface, uvs[a][0], uvs[a][1], face.sense);
                    let nb = eval_surface_normal(&face.surface, uvs[b][0], uvs[b][1], face.sense);
                    let nc = eval_surface_normal(&face.surface, uvs[c][0], uvs[c][1], face.sense);
                    let nd = eval_surface_normal(&face.surface, uvs[d][0], uvs[d][1], face.sense);
                    let nav = [
                        na[0] + nb[0] + nc[0],
                        na[1] + nb[1] + nc[1],
                        na[2] + nb[2] + nc[2],
                    ];
                    if dot3(cross3(sub3(pb, pa), sub3(pc, pa)), nav) >= 0.0 {
                        add_tri(pa, pb, pc, na, nb, nc);
                        add_tri(pa, pc, pd, na, nc, nd);
                    } else {
                        add_tri(pa, pc, pb, na, nc, nb);
                        add_tri(pa, pd, pc, na, nd, nc);
                    }
                }
                return;
            }
        }
        let _ = du_span;
    }

    // Fallback: ear-clip directly in UV space using exact 3D points
    let (_, tris) = triangulate_2d_with_holes(vec![uvs.clone()]);
    for [a, b, c] in tris {
        let pa = exact_pts[a];
        let pb = exact_pts[b];
        let pc = exact_pts[c];
        let na = eval_surface_normal(&face.surface, uvs[a][0], uvs[a][1], face.sense);
        let nb = eval_surface_normal(&face.surface, uvs[b][0], uvs[b][1], face.sense);
        let nc = eval_surface_normal(&face.surface, uvs[c][0], uvs[c][1], face.sense);
        let nav = [na[0] + nb[0] + nc[0], na[1] + nb[1] + nc[1], na[2] + nb[2] + nc[2]];
        if dot3(cross3(sub3(pb, pa), sub3(pc, pa)), nav) >= 0.0 {
            add_tri(pa, pb, pc, na, nb, nc);
        } else {
            add_tri(pa, pc, pb, na, nc, nb);
        }
    }
}

fn quantize_mm(p: [f64; 3]) -> [i64; 3] {
    let s = 1e6; // micrometer grid for vertex welding
    [
        (p[0] * s).round() as i64,
        (p[1] * s).round() as i64,
        (p[2] * s).round() as i64,
    ]
}

fn append_transformed_body(
    model: &BrepModel,
    shell_set: &BTreeSet<i64>,
    body_app: &Appearance,
    tf: Option<&Transform3D>,
    n_arc_full: usize,
    out: &mut TessMesh,
    weld_map: &mut BTreeMap<[i64; 3], u32>,
) {
    let det_neg = tf.is_some_and(|t| t.det() < 0.0);
    let default_rgb = [0.68f32, 0.74, 0.82];

    let xform_pt = |p: [f64; 3]| -> [f64; 3] {
        match tf {
            Some(t) => t.apply_point(p),
            None => p,
        }
    };

    // Pre-sample every edge referenced by faces in this body ONCE
    let mut sampled_edges: BTreeMap<i64, Vec<[f64; 3]>> = BTreeMap::new();
    let mut seen_edges_for_lines = BTreeSet::new();

    for f in model.faces.values() {
        if !f.shell_index.is_some_and(|si| shell_set.contains(&si)) {
            continue;
        }
        for lp in &f.loops {
            for ce in &lp.coedges {
                let Some(edge) = model.edges.get(&ce.edge_index) else {
                    continue;
                };
                if sampled_edges.contains_key(&edge.index) {
                    continue;
                }
                let (sv_idx, ev_idx) = if ce.sense.is_plus() {
                    (ce.start_vertex, ce.end_vertex)
                } else {
                    (ce.end_vertex, ce.start_vertex)
                };
                let p_s = sv_idx
                    .and_then(|i| model.vertices.get(&i))
                    .map(|v| xform_pt(v.point))
                    .unwrap_or([0.0; 3]);
                let p_e = ev_idx
                    .and_then(|i| model.vertices.get(&i))
                    .map(|v| xform_pt(v.point))
                    .unwrap_or(p_s);
                let curve_xf = match tf {
                    Some(t) => transform_curve_data(&edge.curve, t),
                    None => edge.curve.clone(),
                };
                let pts = sample_edge_points(&curve_xf, p_s, p_e, edge.sense.is_plus(), n_arc_full);
                if seen_edges_for_lines.insert(edge.index) {
                    for w in pts.windows(2) {
                        out.edge_lines.push((w[0][0] * 1000.0) as f32);
                        out.edge_lines.push((w[0][1] * 1000.0) as f32);
                        out.edge_lines.push((w[0][2] * 1000.0) as f32);
                        out.edge_lines.push((w[1][0] * 1000.0) as f32);
                        out.edge_lines.push((w[1][1] * 1000.0) as f32);
                        out.edge_lines.push((w[1][2] * 1000.0) as f32);
                    }
                }
                sampled_edges.insert(edge.index, pts);
            }
        }
    }

    for f in model.faces.values() {
        if !f.shell_index.is_some_and(|si| shell_set.contains(&si)) {
            continue;
        }
        let mut loops_3d = Vec::with_capacity(f.loops.len());
        for lp in &f.loops {
            let mut ring = Vec::new();
            for ce in &lp.coedges {
                let Some(epts) = sampled_edges.get(&ce.edge_index) else {
                    continue;
                };
                if ce.sense.is_plus() {
                    for &p in epts.iter().take(epts.len().saturating_sub(1)) {
                        ring.push(p);
                    }
                } else {
                    for &p in epts.iter().rev().take(epts.len().saturating_sub(1)) {
                        ring.push(p);
                    }
                }
            }
            if ring.len() >= 3 {
                loops_3d.push(ring);
            }
        }

        let eff_face = Face {
            index: f.index,
            node_id: f.node_id,
            shell_index: f.shell_index,
            sense: if det_neg {
                if f.sense.is_plus() {
                    crate::model::Sense::Minus
                } else {
                    crate::model::Sense::Plus
                }
            } else {
                f.sense
            },
            face_sense: f.face_sense,
            surface_sense: f.surface_sense,
            surface_index: f.surface_index,
            surface: match tf {
                Some(t) => transform_surface_data(&f.surface, t),
                None => f.surface.clone(),
            },
            loops: Vec::new(),
            tolerance: f.tolerance,
            appearance: f.appearance.clone(),
        };

        let rgb = f
            .appearance
            .color_rgb
            .or(body_app.color_rgb)
            .map(|c| [c[0] as f32, c[1] as f32, c[2] as f32])
            .unwrap_or(default_rgb);

        let mut add_tri = |pa: [f64; 3],
                           pb: [f64; 3],
                           pc: [f64; 3],
                           na: [f64; 3],
                           nb: [f64; 3],
                           nc: [f64; 3]| {
            let base_r = (out.render_positions.len() / 3) as u32;
            let pts_m = [pa, pb, pc];
            let nrms = [na, nb, nc];
            let mut m_idx = [0u32; 3];
            for k in 0..3 {
                let p_mm = [
                    pts_m[k][0] * 1000.0,
                    pts_m[k][1] * 1000.0,
                    pts_m[k][2] * 1000.0,
                ];
                out.render_positions.push(p_mm[0] as f32);
                out.render_positions.push(p_mm[1] as f32);
                out.render_positions.push(p_mm[2] as f32);
                out.render_normals.push(nrms[k][0] as f32);
                out.render_normals.push(nrms[k][1] as f32);
                out.render_normals.push(nrms[k][2] as f32);
                out.render_colors.push(rgb[0]);
                out.render_colors.push(rgb[1]);
                out.render_colors.push(rgb[2]);
                out.render_indices.push(base_r + k as u32);

                let qk = quantize_mm(p_mm);
                let wid = *weld_map.entry(qk).or_insert_with(|| {
                    let id = out.mass_positions_mm.len() as u32;
                    out.mass_positions_mm.push(p_mm);
                    id
                });
                m_idx[k] = wid;
            }
            if m_idx[0] != m_idx[1] && m_idx[1] != m_idx[2] && m_idx[0] != m_idx[2] {
                out.mass_triangles.push(m_idx);
            }
        };

        tessellate_analytic_or_nurbs_face(&eff_face, &loops_3d, &mut add_tri);
    }
}

pub fn tessellate_brep(model: &BrepModel, n_arc_full: usize) -> TessMesh {
    let mut out = TessMesh {
        render_positions: Vec::new(),
        render_normals: Vec::new(),
        render_colors: Vec::new(),
        render_indices: Vec::new(),
        edge_lines: Vec::new(),
        mass_positions_mm: Vec::new(),
        mass_triangles: Vec::new(),
    };
    let mut weld_map: BTreeMap<[i64; 3], u32> = BTreeMap::new();
    let n_seg = n_arc_full.max(16);

    let mut did_asm = false;
    if !model.assemblies.is_empty() && !model.instances.is_empty() {
        let asm_map: BTreeMap<i64, &AssemblyInfo> =
            model.assemblies.iter().map(|a| (a.index, a)).collect();
        let body_map: BTreeMap<i64, &BodyInfo> =
            model.bodies.iter().map(|b| (b.index, b)).collect();
        let inst_map: BTreeMap<i64, _> = model.instances.iter().map(|i| (i.index, i)).collect();
        let referenced_asms: BTreeSet<i64> = model
            .instances
            .iter()
            .filter_map(|i| i.part_index.filter(|pi| asm_map.contains_key(pi)))
            .collect();

        fn visit_asm(
            asm: &AssemblyInfo,
            parent_tf: &Transform3D,
            parent_app: &Appearance,
            asm_map: &BTreeMap<i64, &AssemblyInfo>,
            body_map: &BTreeMap<i64, &BodyInfo>,
            inst_map: &BTreeMap<i64, &crate::model::InstanceInfo>,
            model: &BrepModel,
            n_seg: usize,
            out: &mut TessMesh,
            weld_map: &mut BTreeMap<[i64; 3], u32>,
            visiting: &mut BTreeSet<i64>,
            placed_any: &mut bool,
        ) {
            if !visiting.insert(asm.index) {
                return;
            }
            for &ii in &asm.instance_indices {
                let Some(&inst) = inst_map.get(&ii) else {
                    continue;
                };
                let Some(pi) = inst.part_index else {
                    continue;
                };
                let cur_tf = match &inst.transform {
                    Some(t) => parent_tf.compose(t),
                    None => parent_tf.clone(),
                };
                let cur_app = inst.appearance.inherit_from(parent_app);
                if let Some(&body) = body_map.get(&pi) {
                    let sh_set: BTreeSet<i64> = body.shell_indices.iter().copied().collect();
                    let app = body.appearance.inherit_from(&cur_app);
                    append_transformed_body(
                        model,
                        &sh_set,
                        &app,
                        Some(&cur_tf),
                        n_seg,
                        out,
                        weld_map,
                    );
                    *placed_any = true;
                } else if let Some(&sub_asm) = asm_map.get(&pi) {
                    visit_asm(
                        sub_asm,
                        &cur_tf,
                        &cur_app,
                        asm_map,
                        body_map,
                        inst_map,
                        model,
                        n_seg,
                        out,
                        weld_map,
                        visiting,
                        placed_any,
                    );
                }
            }
            visiting.remove(&asm.index);
        }

        let roots: Vec<&AssemblyInfo> = model
            .assemblies
            .iter()
            .filter(|a| !referenced_asms.contains(&a.index))
            .collect();
        let targets = if roots.is_empty() {
            model.assemblies.iter().collect()
        } else {
            roots
        };
        let mut visiting = BTreeSet::new();
        for asm in targets {
            visit_asm(
                asm,
                &Transform3D::identity(),
                &asm.appearance,
                &asm_map,
                &body_map,
                &inst_map,
                model,
                n_seg,
                &mut out,
                &mut weld_map,
                &mut visiting,
                &mut did_asm,
            );
        }
    }

    if !did_asm {
        if !model.bodies.is_empty() {
            for b in &model.bodies {
                let sh_set: BTreeSet<i64> = b.shell_indices.iter().copied().collect();
                append_transformed_body(
                    model,
                    &sh_set,
                    &b.appearance,
                    None,
                    n_seg,
                    &mut out,
                    &mut weld_map,
                );
            }
        } else {
            let mut sh_set = BTreeSet::new();
            for f in model.faces.values() {
                if let Some(si) = f.shell_index {
                    sh_set.insert(si);
                }
            }
            append_transformed_body(
                model,
                &sh_set,
                &Appearance::default(),
                None,
                n_seg,
                &mut out,
                &mut weld_map,
            );
        }
    }

    out
}

pub fn write_stl_binary_bytes(mesh: &TessMesh) -> Vec<u8> {
    let n_tris = mesh.render_indices.len() / 3;
    let mut out = Vec::with_capacity(84 + n_tris * 50);
    let mut header = [0u8; 80];
    let tag = b"xtopen clean-room binary STL (mm)";
    header[..tag.len()].copy_from_slice(tag);
    out.extend_from_slice(&header);
    out.extend_from_slice(&(n_tris as u32).to_le_bytes());

    let p = &mesh.render_positions;
    for t in mesh.render_indices.chunks_exact(3) {
        let ia = t[0] as usize * 3;
        let ib = t[1] as usize * 3;
        let ic = t[2] as usize * 3;
        let a = [p[ia] as f64, p[ia + 1] as f64, p[ia + 2] as f64];
        let b = [p[ib] as f64, p[ib + 1] as f64, p[ib + 2] as f64];
        let c = [p[ic] as f64, p[ic + 1] as f64, p[ic + 2] as f64];
        let n = norm3(cross3(sub3(b, a), sub3(c, a)));
        for val in [
            n[0] as f32,
            n[1] as f32,
            n[2] as f32,
            a[0] as f32,
            a[1] as f32,
            a[2] as f32,
            b[0] as f32,
            b[1] as f32,
            b[2] as f32,
            c[0] as f32,
            c[1] as f32,
            c[2] as f32,
        ] {
            out.extend_from_slice(&val.to_le_bytes());
        }
        out.extend_from_slice(&0u16.to_le_bytes());
    }
    out
}

pub fn write_obj_string(mesh: &TessMesh) -> String {
    let mut out = String::with_capacity(mesh.mass_positions_mm.len() * 32 + mesh.mass_triangles.len() * 24 + 64);
    out.push_str("# xtopen clean-room OBJ export (mm)\no model\n");
    for v in &mesh.mass_positions_mm {
        let _ = writeln!(out, "v {:.6} {:.6} {:.6}", v[0], v[1], v[2]);
    }
    for &[a, b, c] in &mesh.mass_triangles {
        let _ = writeln!(out, "f {} {} {}", a + 1, b + 1, c + 1);
    }
    out
}
