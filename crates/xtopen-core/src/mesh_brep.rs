//! Clean-room triangle mesh to watertight planar B-rep conversion (with coplanar merging).

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use crate::model::{
    Appearance, BodyInfo, BrepModel, Coedge, CurveData, CurveKind, Edge, Face, Loop, Sense,
    SurfaceData, SurfaceKind, Vertex,
};
use crate::xb::{XtError, XtHeader};

pub struct MeshBodyInput<'a> {
    pub name: &'a str,
    /// Triangle vertex positions in **millimeters** (SolidCraft convention).
    pub positions_mm: &'a [[f64; 3]],
    /// Triangle vertex index triples.
    pub triangles: &'a [[u32; 3]],
    pub appearance: Option<Appearance>,
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

fn len3(v: [f64; 3]) -> f64 {
    dot3(v, v).sqrt()
}

fn norm3(v: [f64; 3]) -> [f64; 3] {
    let l = len3(v);
    if l < 1e-15 {
        [0.0, 0.0, 1.0]
    } else {
        [v[0] / l, v[1] / l, v[2] / l]
    }
}

fn quantize_m(p: [f64; 3]) -> [i64; 3] {
    let s = 1e9; // nanometer welding grid
    [
        (p[0] * s).round() as i64,
        (p[1] * s).round() as i64,
        (p[2] * s).round() as i64,
    ]
}

struct WeldedMesh {
    verts: Vec<[f64; 3]>,
    tris: Vec<[usize; 3]>,
    normals: Vec<[f64; 3]>,
    planes_d: Vec<f64>,
}

fn weld_mesh_meters(positions_mm: &[[f64; 3]], triangles: &[[u32; 3]]) -> WeldedMesh {
    let mut map: BTreeMap<[i64; 3], usize> = BTreeMap::new();
    let mut verts: Vec<[f64; 3]> = Vec::new();
    let mut orig_to_welded = Vec::with_capacity(positions_mm.len());
    for &pm in positions_mm {
        let p = [pm[0] * 0.001, pm[1] * 0.001, pm[2] * 0.001];
        let key = quantize_m(p);
        let wid = *map.entry(key).or_insert_with(|| {
            let id = verts.len();
            verts.push(p);
            id
        });
        orig_to_welded.push(wid);
    }

    let mut tris = Vec::with_capacity(triangles.len());
    let mut normals = Vec::with_capacity(triangles.len());
    let mut planes_d = Vec::with_capacity(triangles.len());
    for &[i0, i1, i2] in triangles {
        let a = orig_to_welded[i0 as usize];
        let b = orig_to_welded[i1 as usize];
        let c = orig_to_welded[i2 as usize];
        if a == b || b == c || a == c {
            continue;
        }
        let n_raw = cross3(sub3(verts[b], verts[a]), sub3(verts[c], verts[a]));
        let l = len3(n_raw);
        if l < 1e-18 {
            continue;
        }
        let n = [n_raw[0] / l, n_raw[1] / l, n_raw[2] / l];
        let d = dot3(n, verts[a]);
        tris.push([a, b, c]);
        normals.push(n);
        planes_d.push(d);
    }

    WeldedMesh {
        verts,
        tris,
        normals,
        planes_d,
    }
}

fn ortho_x(n: [f64; 3], hint: [f64; 3]) -> [f64; 3] {
    let d = dot3(hint, n);
    let xp = [hint[0] - d * n[0], hint[1] - d * n[1], hint[2] - d * n[2]];
    if len3(xp) > 1e-10 {
        return norm3(xp);
    }
    let cand = if n[0].abs() < 0.9 {
        [1.0, 0.0, 0.0]
    } else {
        [0.0, 1.0, 0.0]
    };
    norm3(cross3(n, cand))
}

fn append_mesh_body(
    mesh: &MeshBodyInput<'_>,
    merge_coplanar: bool,
    model: &mut BrepModel,
    next_idx: &mut i64,
) -> Result<(), XtError> {
    let wm = weld_mesh_meters(mesh.positions_mm, mesh.triangles);
    if wm.tris.is_empty() {
        return Err(XtError::InvalidInput(format!(
            "Mesh for '{}' has no non-degenerate triangles",
            mesh.name
        )));
    }

    // Build directed edge map: (u, v) -> owning triangle index
    let mut dir_edge_tri: BTreeMap<(usize, usize), usize> = BTreeMap::new();
    for (ti, &[a, b, c]) in wm.tris.iter().enumerate() {
        dir_edge_tri.insert((a, b), ti);
        dir_edge_tri.insert((b, c), ti);
        dir_edge_tri.insert((c, a), ti);
    }

    // Check watertightness: every (u, v) should have opposite (v, u)
    let is_closed = dir_edge_tri
        .keys()
        .all(|&(u, v)| dir_edge_tri.contains_key(&(v, u)));

    // Group triangles into planar patches
    let n_tris = wm.tris.len();
    let mut tri_patch = vec![usize::MAX; n_tris];
    let mut patches: Vec<(Vec<usize>, [f64; 3])> = Vec::new();

    if merge_coplanar {
        let mut q = VecDeque::new();
        for start in 0..n_tris {
            if tri_patch[start] != usize::MAX {
                continue;
            }
            let pid = patches.len();
            let ref_n = wm.normals[start];
            let ref_d = wm.planes_d[start];
            let mut members = Vec::new();
            let mut n_sum = [0.0, 0.0, 0.0];
            tri_patch[start] = pid;
            q.push_back(start);

            while let Some(ti) = q.pop_front() {
                members.push(ti);
                let tri = wm.tris[ti];
                let cr = cross3(
                    sub3(wm.verts[tri[1]], wm.verts[tri[0]]),
                    sub3(wm.verts[tri[2]], wm.verts[tri[0]]),
                );
                n_sum[0] += cr[0];
                n_sum[1] += cr[1];
                n_sum[2] += cr[2];

                for k in 0..3 {
                    let u = tri[k];
                    let v = tri[(k + 1) % 3];
                    if let Some(&nbr) = dir_edge_tri.get(&(v, u)) {
                        if tri_patch[nbr] == usize::MAX {
                            let nn = wm.normals[nbr];
                            if dot3(nn, ref_n) > 1.0 - 1e-7 {
                                let nt = wm.tris[nbr];
                                let ok_plane = nt
                                    .iter()
                                    .all(|&vi| (dot3(ref_n, wm.verts[vi]) - ref_d).abs() < 1e-6);
                                if ok_plane {
                                    tri_patch[nbr] = pid;
                                    q.push_back(nbr);
                                }
                            }
                        }
                    }
                }
            }

            let avg_n = if len3(n_sum) > 1e-18 {
                norm3(n_sum)
            } else {
                ref_n
            };
            patches.push((members, avg_n));
        }
    } else {
        for ti in 0..n_tris {
            tri_patch[ti] = ti;
            patches.push((vec![ti], wm.normals[ti]));
        }
    }

    // Trace directed boundary cycles per patch
    let mut patch_cycles: Vec<(Vec<Vec<usize>>, [f64; 3])> = Vec::with_capacity(patches.len());
    for (pid, (members, p_normal)) in patches.iter().enumerate() {
        let mut out_edges: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
        for &ti in members {
            let tri = wm.tris[ti];
            for k in 0..3 {
                let u = tri[k];
                let v = tri[(k + 1) % 3];
                let same_patch = dir_edge_tri
                    .get(&(v, u))
                    .is_some_and(|&nbr| tri_patch[nbr] == pid);
                if !same_patch {
                    out_edges.entry(u).or_default().push(v);
                }
            }
        }

        // Remove antiparallel interior slits if any exist (should not for simple patches)
        let x_ax = ortho_x(*p_normal, [1.0, 0.0, 0.0]);
        let y_ax = cross3(*p_normal, x_ax);
        let mut cycles = Vec::new();

        loop {
            let Some((&start_u, nbrs)) = out_edges.iter().find(|(_, vs)| !vs.is_empty()) else {
                break;
            };
            let first_v = nbrs[0];
            let mut cur_u = start_u;
            let mut cur_v = first_v;
            // Consume start_u -> first_v
            if let Some(vs) = out_edges.get_mut(&cur_u) {
                if let Some(pos) = vs.iter().position(|&x| x == cur_v) {
                    vs.remove(pos);
                }
            }
            let mut ring = vec![cur_u];
            let mut guard = 0usize;
            while cur_v != start_u && guard < wm.verts.len() + 16 {
                guard += 1;
                ring.push(cur_v);
                let prev_dir = norm3(sub3(wm.verts[cur_u], wm.verts[cur_v]));
                let prev_ang = dot3(prev_dir, y_ax).atan2(dot3(prev_dir, x_ax));
                let Some(cands) = out_edges.get_mut(&cur_v) else {
                    break;
                };
                if cands.is_empty() {
                    break;
                }
                // Pick next outgoing half-edge turning tightest CW around vertex relative to previous incoming edge
                let mut best_i = 0usize;
                let mut best_delta = f64::INFINITY;
                for (ci, &nv) in cands.iter().enumerate() {
                    let nd = norm3(sub3(wm.verts[nv], wm.verts[cur_v]));
                    let ang = dot3(nd, y_ax).atan2(dot3(nd, x_ax));
                    let mut delta = ang - prev_ang;
                    while delta <= 0.0 {
                        delta += std::f64::consts::TAU;
                    }
                    if delta < best_delta {
                        best_delta = delta;
                        best_i = ci;
                    }
                }
                let next_v = cands.remove(best_i);
                cur_u = cur_v;
                cur_v = next_v;
            }
            if ring.len() >= 3 && cur_v == start_u {
                cycles.push(ring);
            }
        }
        patch_cycles.push((cycles, *p_normal));
    }

    // Identify global corner vertices so edge simplification never breaks shared topology
    let mut corner_verts: BTreeSet<usize> = BTreeSet::new();
    if merge_coplanar {
        let mut vert_patches: BTreeMap<usize, BTreeSet<usize>> = BTreeMap::new();
        for (pid, (cycles, _)) in patch_cycles.iter().enumerate() {
            for cyc in cycles {
                for &vi in cyc {
                    vert_patches.entry(vi).or_default().insert(pid);
                }
            }
        }
        for (vi, ps) in &vert_patches {
            if ps.len() != 2 {
                corner_verts.insert(*vi);
            }
        }
        for (cycles, _) in &patch_cycles {
            for cyc in cycles {
                let m = cyc.len();
                for i in 0..m {
                    let va = cyc[(i + m - 1) % m];
                    let vb = cyc[i];
                    let vc = cyc[(i + 1) % m];
                    if corner_verts.contains(&vb) {
                        continue;
                    }
                    let d1 = norm3(sub3(wm.verts[vb], wm.verts[va]));
                    let d2 = norm3(sub3(wm.verts[vc], wm.verts[vb]));
                    if dot3(d1, d2) < 1.0 - 1e-7 {
                        corner_verts.insert(vb);
                    }
                }
            }
        }
        // Keep only corner vertices on every cycle (ensure >= 3 corners remain per cycle)
        for (cycles, _) in &mut patch_cycles {
            for cyc in cycles.iter_mut() {
                let kept: Vec<usize> = cyc
                    .iter()
                    .copied()
                    .filter(|vi| corner_verts.contains(vi))
                    .collect();
                if kept.len() >= 3 {
                    *cyc = kept;
                } else {
                    for &vi in cyc.iter() {
                        corner_verts.insert(vi);
                    }
                }
            }
        }
    } else {
        for (cycles, _) in &patch_cycles {
            for cyc in cycles {
                for &vi in cyc {
                    corner_verts.insert(vi);
                }
            }
        }
    }

    // Emit BrepModel entities
    let mut alloc = || {
        let i = *next_idx;
        *next_idx += 1;
        i
    };

    let body_idx = alloc();
    let shell_idx = alloc();

    let mut vert_to_idx: BTreeMap<usize, i64> = BTreeMap::new();
    for &vi in &corner_verts {
        let vid = alloc();
        vert_to_idx.insert(vi, vid);
        model.vertices.insert(
            vid,
            Vertex {
                index: vid,
                node_id: vid,
                point: wm.verts[vi],
                tolerance: 1e-7,
            },
        );
    }

    // Undirected canonical edge map (min(a,b), max(a,b)) -> (edge_idx, start_v, end_v)
    let mut edge_map: BTreeMap<(usize, usize), (i64, usize, usize)> = BTreeMap::new();
    for (cycles, p_normal) in &patch_cycles {
        let usable_cycles: Vec<&Vec<usize>> = cycles.iter().filter(|c| c.len() >= 3).collect();
        if usable_cycles.is_empty() {
            continue;
        }
        let fi = alloc();
        let origin = wm.verts[usable_cycles[0][0]];
        let first_dir = norm3(sub3(wm.verts[usable_cycles[0][1]], origin));
        let x_axis = ortho_x(*p_normal, first_dir);
        let surf = SurfaceData {
            sense: Sense::Plus,
            kind: SurfaceKind::Plane {
                origin,
                normal: *p_normal,
                x_axis,
            },
        };
        let mut loops = Vec::with_capacity(usable_cycles.len());
        for cyc in usable_cycles {
            let li = alloc();
            let mut coedges = Vec::with_capacity(cyc.len());
            for k in 0..cyc.len() {
                let u = cyc[k];
                let v = cyc[(k + 1) % cyc.len()];
                let key = if u < v { (u, v) } else { (v, u) };
                let (e_idx, e_start, _e_end) = *edge_map.entry(key).or_insert_with(|| {
                    let eid = alloc();
                    let p0 = wm.verts[u];
                    let dir = norm3(sub3(wm.verts[v], p0));
                    model.edges.insert(
                        eid,
                        Edge {
                            index: eid,
                            node_id: eid,
                            sense: Sense::Plus,
                            tolerance: 1e-7,
                            curve_index: None,
                            curve: CurveData {
                                sense: Sense::Plus,
                                kind: CurveKind::Line { origin: p0, dir },
                                sp_surface: None,
                            },
                        },
                    );
                    (eid, u, v)
                });
                let sense = if u == e_start { Sense::Plus } else { Sense::Minus };
                let fin_idx = alloc();
                coedges.push(Coedge {
                    edge_index: e_idx,
                    sense,
                    start_vertex: Some(vert_to_idx[&u]),
                    end_vertex: Some(vert_to_idx[&v]),
                    fin_index: fin_idx,
                });
            }
            loops.push(Loop { index: li, coedges });
        }
        model.faces.insert(
            fi,
            Face {
                index: fi,
                node_id: fi,
                shell_index: Some(shell_idx),
                sense: Sense::Plus,
                face_sense: Sense::Plus,
                surface_sense: Sense::Plus,
                surface_index: None,
                surface: surf,
                loops,
                tolerance: 1e-7,
                appearance: Appearance::default(),
            },
        );
    }

    let face_count = model
        .faces
        .values()
        .filter(|f| f.shell_index == Some(shell_idx))
        .count();
    let app = mesh.appearance.clone().unwrap_or_else(|| Appearance {
        name: if mesh.name.is_empty() {
            None
        } else {
            Some(mesh.name.to_string())
        },
        ..Default::default()
    });
    model.bodies.push(BodyInfo {
        index: body_idx,
        node_id: body_idx,
        body_type: Some(if is_closed { 1 } else { 2 }),
        shell_indices: vec![shell_idx],
        face_count,
        name: app.name.clone(),
        appearance: app,
    });
    Ok(())
}

pub fn meshes_to_brep(meshes: &[MeshBodyInput<'_>], merge_coplanar: bool) -> Result<BrepModel, XtError> {
    if meshes.is_empty() {
        return Err(XtError::Unsupported(
            "No solid or sheet bodies available to export".into(),
        ));
    }
    let mut model = BrepModel {
        header: XtHeader {
            format_kind: "binary".to_string(),
            modeler: " : TRANSMIT FILE created by xtopen".to_string(),
            schema_name: "sch_12000".to_string(),
            ..Default::default()
        },
        bodies: Vec::new(),
        vertices: BTreeMap::new(),
        edges: BTreeMap::new(),
        faces: BTreeMap::new(),
        surfaces: BTreeMap::new(),
        curves: BTreeMap::new(),
        assemblies: Vec::new(),
        instances: Vec::new(),
        attributes: Vec::new(),
        warnings: Vec::new(),
    };
    let mut next_idx = 1i64;
    for m in meshes {
        append_mesh_body(m, merge_coplanar, &mut model, &mut next_idx)?;
    }
    if model.faces.is_empty() {
        return Err(XtError::Unsupported("Resulting B-rep has no faces".into()));
    }
    Ok(model)
}
