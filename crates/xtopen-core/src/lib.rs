//! Clean-room pure-Rust Parasolid XT parser, B-rep model extractor, and STEP AP214/AP242 writer.

pub mod model;
pub mod schema;
pub mod step_writer;
pub mod xb;
pub mod xt;

pub use model::{
    extract_brep, BodyInfo, BrepModel, Coedge, ConversionWarning, CurveData, CurveKind, Edge, Face,
    Loop, NurbsCurve, NurbsSurface, Sense, SurfaceData, SurfaceKind, Vertex,
};
pub use step_writer::{convert_parsed_to_step, dumps_step, StepOptions, StepSchema, StepUnits};
pub use xb::{parse_xb, ParseResult, XtError, XtHeader, XtNode, XtValue};
pub use xt::parse_xt;

#[derive(Clone, Debug)]
pub struct ConvertOutput {
    pub step: String,
    pub bodies: usize,
    pub faces: usize,
    pub edges: usize,
    pub vertices: usize,
    pub format_kind: String,
    pub schema_name: String,
    pub warnings: Vec<ConversionWarning>,
}

/// Auto-detect binary (.x_b / .x_p) vs ASCII text (.x_t) Parasolid XT bytes and parse them.
pub fn parse_bytes(data: &[u8]) -> Result<ParseResult, XtError> {
    if data.is_empty() {
        return Err(XtError::InvalidInput("Input Parasolid file is empty".into()));
    }
    if data.starts_with(b"PS\x00\x00") {
        return parse_xb(data);
    }
    if let Some(pos) = find_subslice(data, b"**END_OF_HEADER") {
        let nl = data[pos..]
            .iter()
            .position(|&b| b == b'\n')
            .map(|p| pos + p + 1)
            .unwrap_or(data.len());
        let mut body_start = nl;
        while body_start < data.len() && matches!(data[body_start], b'\r' | b'\n') {
            body_start += 1;
        }
        let body = &data[body_start..];
        if body.starts_with(b"PS\x00\x00") {
            return parse_xb(data);
        }
        let trimmed = body
            .iter()
            .position(|b| !matches!(b, b' ' | b'\t' | b'\r' | b'\n'))
            .unwrap_or(body.len());
        if body[trimmed..].starts_with(b"T") {
            return parse_xt(data);
        }
    }
    let trimmed = data
        .iter()
        .position(|b| !matches!(b, b' ' | b'\t' | b'\r' | b'\n'))
        .unwrap_or(data.len());
    if data[trimmed..].starts_with(b"T") {
        return parse_xt(data);
    }
    parse_xb(data)
}

pub fn convert_bytes_to_step(data: &[u8], opts: &StepOptions) -> Result<ConvertOutput, XtError> {
    let parsed = parse_bytes(data)?;
    let model = extract_brep(&parsed);
    if model.faces.is_empty() {
        return Err(XtError::Unsupported(
            "Parasolid file contains no B-rep faces to import".into(),
        ));
    }
    let step = dumps_step(&model, opts);
    Ok(ConvertOutput {
        step,
        bodies: model.bodies.len(),
        faces: model.faces.len(),
        edges: model.edges.len(),
        vertices: model.vertices.len(),
        format_kind: model.header.format_kind.clone(),
        schema_name: model.header.schema_name.clone(),
        warnings: model.warnings.clone(),
    })
}

fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || needle.len() > haystack.len() {
        return None;
    }
    haystack.windows(needle.len()).position(|w| w == needle)
}
