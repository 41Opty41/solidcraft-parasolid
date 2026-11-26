use std::path::Path;
use xtopen_core::{
    convert_bytes_to_step, extract_brep, parse_bytes, StepOptions, StepSchema, StepUnits, XtError,
};

#[test]
fn parses_and_exports_synthetic_xb_and_xt() {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let tests_dir = manifest_dir
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("solidcraft-parasolid")
        .join("tests");

    assert!(tests_dir.exists(), "missing fixtures dir: {}", tests_dir.display());

    let names = [
        "box.x_b",
        "box.x_t",
        "cylinder.x_b",
        "cone_frustum.x_b",
        "plate_with_hole.x_b",
        "sphere.x_b",
        "two_boxes.x_b",
        "nurbs_sheet.x_b",
    ];
    let mut checked = 0usize;
    for name in names {
        let p = tests_dir.join(name);
        assert!(p.exists(), "missing fixture: {}", p.display());
        let bytes = std::fs::read(&p).unwrap();
        let parsed = parse_bytes(&bytes).unwrap_or_else(|e| panic!("failed parsing {name}: {e}"));
        let model = extract_brep(&parsed);
        assert!(!model.faces.is_empty(), "{name} had no faces");
        let out = convert_bytes_to_step(
            &bytes,
            &StepOptions {
                schema: StepSchema::Ap214,
                units: StepUnits::Mm,
                name: name.to_string(),
                ..Default::default()
            },
        )
        .unwrap();
        assert!(out.step.starts_with("ISO-10303-21;"));
        assert!(out.step.contains("END-ISO-10303-21;"));
        checked += 1;
    }
    assert_eq!(checked, names.len());
}

#[test]
fn rejects_empty_and_corrupt_inputs() {
    assert!(matches!(parse_bytes(b""), Err(XtError::InvalidInput(_))));
    assert!(matches!(
        parse_bytes(b"NOT_PARASOLID_GARBAGE"),
        Err(XtError::InvalidInput(_))
    ));
}
