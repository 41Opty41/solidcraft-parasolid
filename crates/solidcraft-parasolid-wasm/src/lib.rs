wit_bindgen::generate!({ path: "wit", world: "importer-plugin" });

use craft::plugin::types::{FieldFailure, PluginError};
use craft::plugin::{control, design, source};
use exports::craft::plugin::importer::Guest;
use serde_json::{json, Value};
use xtopen_core::{convert_bytes_to_step, StepOptions, StepSchema, StepUnits, XtError};

struct Importer;

impl Guest for Importer {
    fn import_file(
        _entry: String,
        file: source::FileInfo,
        options: String,
    ) -> Result<String, PluginError> {
        if file.size == 0 {
            return Err(invalid("file", "input Parasolid file is empty"));
        }

        let opts = parse_options(&options)?;
        let mut bytes = Vec::with_capacity(file.size.min(16 << 20) as usize);
        while (bytes.len() as u64) < file.size {
            if control::cancelled() {
                return Err(PluginError::Cancelled);
            }
            let chunk = source::read("", bytes.len() as u64, 1 << 20)?;
            if chunk.is_empty() {
                break;
            }
            bytes.extend_from_slice(&chunk);
        }

        if bytes.is_empty() {
            return Err(invalid("file", "input Parasolid file is empty"));
        }

        let mut step_opts = opts;
        step_opts.name = strip_ext(&file.name);

        let converted = convert_bytes_to_step(&bytes, &step_opts).map_err(|err| match err {
            XtError::InvalidInput(msg) => invalid("file", msg),
            XtError::Unsupported(msg) => PluginError::Unsupported(msg),
        })?;

        let design_doc = json!({
            "format": "solidcraft/67",
            "name": "",
            "units": "mm",
            "params": [],
            "features": [{
                "id": 1,
                "name": "Import1",
                "suppressed": false,
                "body_names": [],
                "param_names": [],
                "type": "import",
                "file": file.name,
                "step": converted.step
            }],
            "next_id": 2,
            "reserved_body_names": [],
            "next_component_color_index": 0
        });

        let payload = design_doc.to_string();
        for chunk in payload.as_bytes().chunks(1 << 20) {
            design::write(chunk)?;
        }

        let loss_items: Vec<Value> = converted
            .warnings
            .iter()
            .map(|w| {
                json!({
                    "severity": "degraded",
                    "title": "Unsupported XT entity substituted",
                    "subject": w.subject,
                    "message": w.message,
                    "code": w.code
                })
            })
            .collect();

        let summary = if converted.assemblies > 0 && converted.instances > 0 {
            format!(
                "Imported {} assembly/assemblies ({} instances), {} body/bodies, {} faces, {} edges from Parasolid ({})",
                converted.assemblies,
                converted.instances,
                converted.bodies,
                converted.faces,
                converted.edges,
                converted.format_kind
            )
        } else {
            format!(
                "Imported {} body/bodies, {} faces, {} edges from Parasolid ({})",
                converted.bodies, converted.faces, converted.edges, converted.format_kind
            )
        };
        Ok(json!({ "summary": summary, "items": loss_items }).to_string())
    }
}

fn parse_options(raw: &str) -> Result<StepOptions, PluginError> {
    let mut opts = StepOptions::default();
    if raw.trim().is_empty() {
        return Ok(opts);
    }
    let v: Value = serde_json::from_str(raw).map_err(|e| invalid("options", e))?;
    if let Some(obj) = v.as_object() {
        if let Some(s) = obj.get("schema").and_then(Value::as_str) {
            opts.schema = match s.to_ascii_uppercase().as_str() {
                "AP214" => StepSchema::Ap214,
                "AP242" => StepSchema::Ap242,
                _ => {
                    return Err(PluginError::Unsupported(format!(
                        "Unsupported STEP schema option: {s}"
                    )));
                }
            };
        }
        if let Some(u) = obj.get("units").and_then(Value::as_str) {
            opts.units = match u {
                "mm" => StepUnits::Mm,
                "m" => StepUnits::M,
                _ => {
                    return Err(PluginError::Unsupported(format!(
                        "Unsupported units option: {u}"
                    )));
                }
            };
        }
    }
    Ok(opts)
}

fn strip_ext(name: &str) -> String {
    match name.rsplit_once('.') {
        Some((stem, _)) if !stem.is_empty() => stem.to_string(),
        _ => {
            if name.is_empty() {
                "xtopen_model".to_string()
            } else {
                name.to_string()
            }
        }
    }
}

fn invalid(field: &str, reason: impl ToString) -> PluginError {
    PluginError::InvalidInput(FieldFailure {
        field: field.into(),
        reason: reason.to_string(),
    })
}

export!(Importer);
