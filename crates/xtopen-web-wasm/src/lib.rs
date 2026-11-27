//! Core wasm32-unknown-unknown C-ABI bridge for browser XT viewer, mass calculator, and client-side CAD export.

use std::cell::RefCell;
use serde_json::json;
use xtopen_core::{
    compute_mesh_mass_props, dumps_step, is_prt_bytes, parse_and_extract_brep, tessellate_brep,
    write_obj_string, write_stl_binary_bytes, write_xb_from_brep, write_xt_from_brep, xb_to_xt,
    xt_to_xb, BrepModel, MeshMassProps, StepOptions, StepSchema, StepUnits, TessMesh,
};

struct LoadedState {
    raw_bytes: Vec<u8>,
    model: BrepModel,
    mesh: TessMesh,
}

thread_local! {
    static STATE: RefCell<Option<LoadedState>> = const { RefCell::new(None) };
    static RESULT_BUF: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
}

fn return_bytes(payload: &[u8]) -> *const u8 {
    RESULT_BUF.with(|buf| {
        let mut b = buf.borrow_mut();
        b.clear();
        b.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        b.extend_from_slice(payload);
        b.as_ptr()
    })
}

fn return_json(v: serde_json::Value) -> *const u8 {
    return_bytes(v.to_string().as_bytes())
}

fn mass_json(props: Option<&MeshMassProps>) -> serde_json::Value {
    match props {
        Some(m) => json!({
            "is_closed": m.is_closed,
            "euler": m.euler_char,
            "volume_mm3": m.volume,
            "volume_cm3": m.volume / 1000.0,
            "area_mm2": m.surface_area,
            "area_cm2": m.surface_area / 100.0,
            "mass_g": m.mass,
            "mass_kg": m.mass / 1000.0,
            "centroid_mm": m.centroid,
            "bbox_min_mm": m.bbox_min,
            "bbox_max_mm": m.bbox_max,
            "principal_moments_g_mm2": m.principal_moments,
            "principal_axes": m.principal_axes,
        }),
        None => json!(null),
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn alloc(len: usize) -> *mut u8 {
    let mut buf = vec![0u8; len];
    let ptr = buf.as_mut_ptr();
    std::mem::forget(buf);
    ptr
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn dealloc(ptr: *mut u8, len: usize) {
    if !ptr.is_null() && len > 0 {
        unsafe {
            let _ = Vec::from_raw_parts(ptr, len, len);
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn load_xt(ptr: *const u8, len: usize, density_kg_m3: f64) -> *const u8 {
    if ptr.is_null() || len == 0 {
        return return_json(json!({ "ok": false, "error": "Input file is empty" }));
    }
    let slice = unsafe { std::slice::from_raw_parts(ptr, len) };
    let model = match parse_and_extract_brep(slice) {
        Ok(m) => m,
        Err(e) => return return_json(json!({ "ok": false, "error": e.to_string() })),
    };
    if model.faces.is_empty() {
        return return_json(
            json!({ "ok": false, "error": "Parasolid file contains no B-rep faces" }),
        );
    }

    let mesh = tessellate_brep(&model, 48);
    // density kg/m^3 -> g/mm^3 is density * 1e-6
    let d_g_mm3 = if density_kg_m3.is_finite() && density_kg_m3 > 0.0 {
        density_kg_m3 * 1e-6
    } else {
        7850.0 * 1e-6
    };
    let props = compute_mesh_mass_props(&mesh.mass_positions_mm, &mesh.mass_triangles, d_g_mm3);

    let body_names: Vec<String> = model
        .bodies
        .iter()
        .enumerate()
        .map(|(i, b)| b.name.clone().unwrap_or_else(|| format!("Body_{}", i + 1)))
        .collect();

    let summary = json!({
        "format": model.header.format_kind,
        "schema": model.header.schema_name,
        "bodies": model.bodies.len(),
        "body_names": body_names,
        "assemblies": model.assemblies.len(),
        "instances": model.instances.len(),
        "faces": model.faces.len(),
        "edges": model.edges.len(),
        "vertices": model.vertices.len(),
        "triangles": mesh.render_indices.len() / 3,
        "warnings": model.warnings.iter().map(|w| w.message.clone()).collect::<Vec<_>>(),
    });

    let buffers = json!({
        "pos_ptr": mesh.render_positions.as_ptr() as usize,
        "pos_len": mesh.render_positions.len(),
        "norm_ptr": mesh.render_normals.as_ptr() as usize,
        "norm_len": mesh.render_normals.len(),
        "col_ptr": mesh.render_colors.as_ptr() as usize,
        "col_len": mesh.render_colors.len(),
        "idx_ptr": mesh.render_indices.as_ptr() as usize,
        "idx_len": mesh.render_indices.len(),
        "edge_ptr": mesh.edge_lines.as_ptr() as usize,
        "edge_len": mesh.edge_lines.len(),
    });

    let m_json = mass_json(props.as_ref());

    STATE.with(|st| {
        *st.borrow_mut() = Some(LoadedState {
            raw_bytes: slice.to_vec(),
            model,
            mesh,
        });
    });

    return_json(json!({
        "ok": true,
        "summary": summary,
        "mass": m_json,
        "buffers": buffers,
    }))
}

#[unsafe(no_mangle)]
pub extern "C" fn recompute_mass(density_kg_m3: f64) -> *const u8 {
    let d_g_mm3 = if density_kg_m3.is_finite() && density_kg_m3 > 0.0 {
        density_kg_m3 * 1e-6
    } else {
        7850.0 * 1e-6
    };
    STATE.with(|st| {
        let guard = st.borrow();
        let Some(ref loaded) = *guard else {
            return return_json(json!({ "ok": false, "error": "No model loaded" }));
        };
        let props =
            compute_mesh_mass_props(&loaded.mesh.mass_positions_mm, &loaded.mesh.mass_triangles, d_g_mm3);
        return_json(json!({ "ok": true, "mass": mass_json(props.as_ref()) }))
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn export_step(schema_code: u32, units_code: u32) -> *const u8 {
    STATE.with(|st| {
        let guard = st.borrow();
        let Some(ref loaded) = *guard else {
            return return_bytes(b"");
        };
        let opts = StepOptions {
            schema: if schema_code == 1 {
                StepSchema::Ap242
            } else {
                StepSchema::Ap214
            },
            units: if units_code == 1 {
                StepUnits::M
            } else {
                StepUnits::Mm
            },
            name: loaded
                .model
                .bodies
                .first()
                .and_then(|b| b.name.clone())
                .unwrap_or_else(|| "xtopen_model".to_string()),
            ..Default::default()
        };
        let step = dumps_step(&loaded.model, &opts);
        return_bytes(step.as_bytes())
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn export_stl() -> *const u8 {
    STATE.with(|st| {
        let guard = st.borrow();
        let Some(ref loaded) = *guard else {
            return return_bytes(b"");
        };
        let bytes = write_stl_binary_bytes(&loaded.mesh);
        return_bytes(&bytes)
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn export_obj() -> *const u8 {
    STATE.with(|st| {
        let guard = st.borrow();
        let Some(ref loaded) = *guard else {
            return return_bytes(b"");
        };
        let obj = write_obj_string(&loaded.mesh);
        return_bytes(obj.as_bytes())
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn transcode_xt(to_binary: u32) -> *const u8 {
    STATE.with(|st| {
        let guard = st.borrow();
        let Some(ref loaded) = *guard else {
            return return_bytes(b"");
        };
        let res = if is_prt_bytes(&loaded.raw_bytes) {
            if to_binary != 0 {
                write_xb_from_brep(&loaded.model)
            } else {
                write_xt_from_brep(&loaded.model)
            }
        } else if to_binary != 0 {
            xt_to_xb(&loaded.raw_bytes)
        } else {
            xb_to_xt(&loaded.raw_bytes)
        };
        match res {
            Ok(bytes) => return_bytes(&bytes),
            Err(_) => return_bytes(b""),
        }
    })
}
