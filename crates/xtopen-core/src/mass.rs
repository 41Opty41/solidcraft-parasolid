//! Clean-room triangle mesh mass properties, inertia tensors, principal axes, and watertightness check.
//! Ported from brepmass (C99) algorithms.

#[derive(Clone, Debug, PartialEq)]
pub struct MeshMassProps {
    pub dim: u8,
    pub is_closed: bool,
    pub euler_char: i64,
    pub n_vertices: usize,
    pub n_triangles: usize,
    pub volume: f64,
    pub surface_area: f64,
    pub mass: f64,
    pub centroid: [f64; 3],
    pub inertia_origin: [[f64; 3]; 3],
    pub inertia_centroid: [[f64; 3]; 3],
    pub principal_moments: [f64; 3],
    pub principal_axes: [[f64; 3]; 3],
    pub bbox_min: [f64; 3],
    pub bbox_max: [f64; 3],
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

fn jacobi3(a_in: [[f64; 3]; 3]) -> ([f64; 3], [[f64; 3]; 3]) {
    let mut a = a_in;
    let mut v = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];

    for _ in 0..50 {
        let mut p = 0usize;
        let mut q = 1usize;
        let mut mx = a[0][1].abs();
        if a[0][2].abs() > mx {
            p = 0;
            q = 2;
            mx = a[0][2].abs();
        }
        if a[1][2].abs() > mx {
            p = 1;
            q = 2;
            mx = a[1][2].abs();
        }
        if mx < 1e-15 {
            break;
        }
        let app = a[p][p];
        let aqq = a[q][q];
        let apq = a[p][q];
        let tau = (aqq - app) / (2.0 * apq);
        let t = if tau >= 0.0 {
            1.0 / (tau + (1.0 + tau * tau).sqrt())
        } else {
            -1.0 / (-tau + (1.0 + tau * tau).sqrt())
        };
        let c = 1.0 / (1.0 + t * t).sqrt();
        let s = t * c;

        a[p][p] = app - t * apq;
        a[q][q] = aqq + t * apq;
        a[p][q] = 0.0;
        a[q][p] = 0.0;

        for r in 0..3 {
            if r == p || r == q {
                continue;
            }
            let arp = a[r][p];
            let arq = a[r][q];
            a[r][p] = c * arp - s * arq;
            a[p][r] = a[r][p];
            a[r][q] = s * arp + c * arq;
            a[q][r] = a[r][q];
        }
        for r in 0..3 {
            let vrp = v[r][p];
            let vrq = v[r][q];
            v[r][p] = c * vrp - s * vrq;
            v[r][q] = s * vrp + c * vrq;
        }
    }

    let lam = [a[0][0], a[1][1], a[2][2]];
    let mut order = [0usize, 1, 2];
    for i in 0..2 {
        for j in (i + 1)..3 {
            if lam[order[j]] < lam[order[i]] {
                order.swap(i, j);
            }
        }
    }
    let evals = [lam[order[0]], lam[order[1]], lam[order[2]]];
    let mut evecs = [[0.0; 3]; 3];
    for k in 0..3 {
        let col = order[k];
        for r in 0..3 {
            evecs[r][k] = v[r][col];
        }
    }
    (evals, evecs)
}

fn finalize_inertia(
    mass: f64,
    com: [f64; 3],
    cov: [[f64; 3]; 3],
) -> ([[f64; 3]; 3], [[f64; 3]; 3], [f64; 3], [[f64; 3]; 3]) {
    let trace = cov[0][0] + cov[1][1] + cov[2][2];
    let mut i_origin = [[0.0; 3]; 3];
    let mut i_com = [[0.0; 3]; 3];
    for i in 0..3 {
        for j in 0..3 {
            i_origin[i][j] = (if i == j { trace } else { 0.0 }) - cov[i][j];
        }
    }
    let c2 = dot3(com, com);
    for i in 0..3 {
        for j in 0..3 {
            i_com[i][j] = i_origin[i][j] - mass * ((if i == j { c2 } else { 0.0 }) - com[i] * com[j]);
        }
    }
    let (pm, pa) = jacobi3(i_com);
    (i_origin, i_com, pm, pa)
}

pub fn check_mesh_closed(triangles: &[[u32; 3]], n_vertices: usize) -> Option<(bool, i64)> {
    if triangles.is_empty() || n_vertices == 0 {
        return None;
    }
    let mut dir: Vec<(u32, u32)> = Vec::with_capacity(triangles.len() * 3);
    let mut undir: Vec<(u32, u32)> = Vec::with_capacity(triangles.len() * 3);
    let mut used = vec![false; n_vertices];

    for &[v0, v1, v2] in triangles {
        let vs = [v0, v1, v2];
        for e in 0..3 {
            let a = vs[e];
            let b = vs[(e + 1) % 3];
            if (a as usize) >= n_vertices || (b as usize) >= n_vertices {
                return None;
            }
            used[a as usize] = true;
            dir.push((a, b));
            undir.push(if a < b { (a, b) } else { (b, a) });
        }
    }

    dir.sort_unstable();
    undir.sort_unstable();
    undir.dedup();

    let v_used = used.iter().filter(|&&u| u).count() as i64;
    let e_unique = undir.len() as i64;
    let f_count = triangles.len() as i64;
    let euler = v_used - e_unique + f_count;

    let mut closed = true;
    for i in 0..dir.len() {
        if i > 0 && dir[i] == dir[i - 1] {
            closed = false;
            break;
        }
        let rev = (dir[i].1, dir[i].0);
        if dir.binary_search(&rev).is_err() {
            closed = false;
            break;
        }
    }
    Some((closed, euler))
}

pub fn compute_mesh_mass_props(
    vertices: &[[f64; 3]],
    triangles: &[[u32; 3]],
    density: f64,
) -> Option<MeshMassProps> {
    if vertices.is_empty() || triangles.is_empty() || !density.is_finite() || density <= 0.0 {
        return None;
    }
    let (closed, euler) = check_mesh_closed(triangles, vertices.len())?;

    let mut bbox_min = vertices[0];
    let mut bbox_max = vertices[0];
    for p in &vertices[1..] {
        for j in 0..3 {
            if p[j] < bbox_min[j] {
                bbox_min[j] = p[j];
            }
            if p[j] > bbox_max[j] {
                bbox_max[j] = p[j];
            }
        }
    }

    let mut vol = 0.0;
    let mut area = 0.0;
    let mut cm_v = [0.0; 3];
    let mut cm_a = [0.0; 3];
    let mut s_v = [[0.0; 3]; 3];
    let mut s_a = [[0.0; 3]; 3];

    for &[ia, ib, ic] in triangles {
        let a = vertices[ia as usize];
        let b = vertices[ib as usize];
        let c = vertices[ic as usize];
        let e1 = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
        let e2 = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
        let n = cross3(e1, e2);
        let tri_a = 0.5 * dot3(n, n).sqrt();
        area += tri_a;

        let ts = [a[0] + b[0] + c[0], a[1] + b[1] + c[1], a[2] + b[2] + c[2]];
        for i in 0..3 {
            cm_a[i] += tri_a * ts[i] / 3.0;
            for j in 0..3 {
                s_a[i][j] += (tri_a / 12.0) * (a[i] * a[j] + b[i] * b[j] + c[i] * c[j] + ts[i] * ts[j]);
            }
        }

        if closed {
            let cr = cross3(b, c);
            let v = dot3(a, cr) / 6.0;
            vol += v;
            for i in 0..3 {
                cm_v[i] += v * ts[i] / 4.0;
                for j in 0..3 {
                    s_v[i][j] +=
                        (v / 20.0) * (a[i] * a[j] + b[i] * b[j] + c[i] * c[j] + ts[i] * ts[j]);
                }
            }
        }
    }

    if closed {
        if vol < 0.0 {
            vol = -vol;
            for i in 0..3 {
                cm_v[i] = -cm_v[i];
                for j in 0..3 {
                    s_v[i][j] = -s_v[i][j];
                }
            }
        }
        let mass = density * vol;
        let centroid = if vol > 0.0 {
            [cm_v[0] / vol, cm_v[1] / vol, cm_v[2] / vol]
        } else {
            [0.0; 3]
        };
        let mut cov_mass = [[0.0; 3]; 3];
        for i in 0..3 {
            for j in 0..3 {
                cov_mass[i][j] = density * s_v[i][j];
            }
        }
        let (inertia_origin, inertia_centroid, principal_moments, principal_axes) =
            finalize_inertia(mass, centroid, cov_mass);
        Some(MeshMassProps {
            dim: 3,
            is_closed: true,
            euler_char: euler,
            n_vertices: vertices.len(),
            n_triangles: triangles.len(),
            volume: vol,
            surface_area: area,
            mass,
            centroid,
            inertia_origin,
            inertia_centroid,
            principal_moments,
            principal_axes,
            bbox_min,
            bbox_max,
        })
    } else {
        let mass = density * area;
        let centroid = if area > 0.0 {
            [cm_a[0] / area, cm_a[1] / area, cm_a[2] / area]
        } else {
            [0.0; 3]
        };
        let mut cov_mass = [[0.0; 3]; 3];
        for i in 0..3 {
            for j in 0..3 {
                cov_mass[i][j] = density * s_a[i][j];
            }
        }
        let (inertia_origin, inertia_centroid, principal_moments, principal_axes) =
            finalize_inertia(mass, centroid, cov_mass);
        Some(MeshMassProps {
            dim: 2,
            is_closed: false,
            euler_char: euler,
            n_vertices: vertices.len(),
            n_triangles: triangles.len(),
            volume: 0.0,
            surface_area: area,
            mass,
            centroid,
            inertia_origin,
            inertia_centroid,
            principal_moments,
            principal_axes,
            bbox_min,
            bbox_max,
        })
    }
}
