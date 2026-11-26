wit_bindgen::generate!({ path: "../solidcraft-parasolid-wasm/wit", world: "exporter-plugin" });

use craft::plugin::types::{FieldFailure, PluginError};
use craft::plugin::{commands, control};
use exports::craft::plugin::formats_provider::Guest;
use serde_json::{json, Value};
use xtopen_core::{
    meshes_to_brep, write_xb_from_brep, write_xt_from_brep, MeshBodyInput, XtError,
};

struct Exporter;

struct ExportOptions {
    tolerance_mm: f64,
    merge_coplanar: bool,
}

impl Default for ExportOptions {
    fn default() -> Self {
        Self {
            tolerance_mm: 0.05,
            merge_coplanar: true,
        }
    }
}

struct BodyMeshBuf {
    name: String,
    positions: Vec<[f64; 3]>,
    triangles: Vec<[u32; 3]>,
}

impl Guest for Exporter {
    fn export_file(slot: String, options: String) -> Result<Vec<u8>, PluginError> {
        let opts = parse_options(&options)?;

        let doc_raw = commands::run("document.inspect", r#"{"timeline":false}"#)?;
        let doc: Value = serde_json::from_str(&doc_raw)
            .map_err(|e| invalid("document.inspect", e))?;
        let body_names = extract_visible_body_names(&doc);
        if body_names.is_empty() {
            return Err(PluginError::Unsupported(
                "Design contains no visible solid or sheet bodies to export".into(),
            ));
        }

        let mut buffers = Vec::with_capacity(body_names.len());
        for (idx, bname) in body_names.iter().enumerate() {
            if control::cancelled() {
                return Err(PluginError::Cancelled);
            }
            control::progress(
                (idx as f32) / (body_names.len() as f32),
                &format!("Meshing {bname}"),
            );
            let args = json!({ "body": bname, "tolerance": opts.tolerance_mm }).to_string();
            let mesh_raw = commands::run("model.mesh", &args)?;
            let mesh_v: Value = serde_json::from_str(&mesh_raw)
                .map_err(|e| invalid("model.mesh", e))?;
            let (positions, triangles) = parse_mesh_arrays(&mesh_v)?;
            if !positions.is_empty() && !triangles.is_empty() {
                buffers.push(BodyMeshBuf {
                    name: bname.clone(),
                    positions,
                    triangles,
                });
            }
        }

        if buffers.is_empty() {
            return Err(PluginError::Unsupported(
                "Design bodies produced empty meshes".into(),
            ));
        }

        let inputs: Vec<MeshBodyInput<'_>> = buffers
            .iter()
            .map(|b| MeshBodyInput {
                name: &b.name,
                positions_mm: &b.positions,
                triangles: &b.triangles,
                appearance: None,
            })
            .collect();

        let model = meshes_to_brep(&inputs, opts.merge_coplanar).map_err(|e| match e {
            XtError::InvalidInput(msg) => invalid("mesh", msg),
            XtError::Unsupported(msg) => PluginError::Unsupported(msg),
        })?;

        let bytes = match slot.as_str() {
            "export-xt" => write_xt_from_brep(&model),
            _ => write_xb_from_brep(&model),
        }
        .map_err(|e| match e {
            XtError::InvalidInput(msg) => invalid("xt_writer", msg),
            XtError::Unsupported(msg) => PluginError::Unsupported(msg),
        })?;

        Ok(bytes)
    }
}

fn parse_options(raw: &str) -> Result<ExportOptions, PluginError> {
    let mut out = ExportOptions::default();
    if raw.trim().is_empty() {
        return Ok(out);
    }
    let v: Value = serde_json::from_str(raw).map_err(|e| invalid("options", e))?;
    if let Some(obj) = v.as_object() {
        if let Some(q) = obj.get("quality").and_then(Value::as_str) {
            out.tolerance_mm = match q {
                "coarse" => 0.2,
                "standard" => 0.05,
                "fine" => 0.01,
                _ => {
                    return Err(PluginError::Unsupported(format!(
                        "Unsupported mesh quality option: {q}"
                    )));
                }
            };
        }
        if let Some(m) = obj.get("merge_coplanar").and_then(Value::as_bool) {
            out.merge_coplanar = m;
        }
    }
    Ok(out)
}

fn extract_visible_body_names(doc: &Value) -> Vec<String> {
    let mut names = Vec::new();
    let Some(bodies_val) = doc.get("bodies") else {
        return names;
    };
    let items: Option<&Vec<Value>> = if let Some(arr) = bodies_val.as_array() {
        Some(arr)
    } else if let Some(obj) = bodies_val.as_object() {
        obj.get("bodies")
            .or_else(|| obj.get("names"))
            .and_then(Value::as_array)
    } else {
        None
    };

    if let Some(arr) = items {
        for item in arr {
            if let Some(s) = item.as_str() {
                names.push(s.to_string());
            } else if let Some(obj) = item.as_object() {
                if obj.get("visible").and_then(Value::as_bool) == Some(false) {
                    continue;
                }
                if let Some(nm) = obj.get("name").and_then(Value::as_str) {
                    names.push(nm.to_string());
                }
            }
        }
    }
    names
}

fn parse_mesh_arrays(v: &Value) -> Result<(Vec<[f64; 3]>, Vec<[u32; 3]>), PluginError> {
    let pos_arr = v
        .get("positions")
        .and_then(Value::as_array)
        .ok_or_else(|| invalid("model.mesh", "missing positions array"))?;
    let tri_arr = v
        .get("triangles")
        .and_then(Value::as_array)
        .ok_or_else(|| invalid("model.mesh", "missing triangles array"))?;

    let mut positions = Vec::with_capacity(pos_arr.len());
    for p in pos_arr {
        let coords = p
            .as_array()
            .ok_or_else(|| invalid("model.mesh", "position entry is not array"))?;
        if coords.len() < 3 {
            return Err(invalid("model.mesh", "position entry has < 3 floats"));
        }
        let x = coords[0].as_f64().unwrap_or(0.0);
        let y = coords[1].as_f64().unwrap_or(0.0);
        let z = coords[2].as_f64().unwrap_or(0.0);
        positions.push([x, y, z]);
    }

    let mut triangles = Vec::with_capacity(tri_arr.len());
    for t in tri_arr {
        let idxs = t
            .as_array()
            .ok_or_else(|| invalid("model.mesh", "triangle entry is not array"))?;
        if idxs.len() < 3 {
            return Err(invalid("model.mesh", "triangle entry has < 3 indices"));
        }
        let a = idxs[0].as_u64().unwrap_or(0) as u32;
        let b = idxs[1].as_u64().unwrap_or(0) as u32;
        let c = idxs[2].as_u64().unwrap_or(0) as u32;
        triangles.push([a, b, c]);
    }
    Ok((positions, triangles))
}

fn invalid(field: &str, reason: impl ToString) -> PluginError {
    PluginError::InvalidInput(FieldFailure {
        field: field.into(),
        reason: reason.to_string(),
    })
}

export!(Exporter);
