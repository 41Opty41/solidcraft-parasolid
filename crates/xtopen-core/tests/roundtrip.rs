use std::path::Path;
use xtopen_core::{
    compute_mesh_mass_props, convert_bytes_to_step, extract_brep, meshes_to_brep, parse_bytes,
    tessellate_brep, write_obj_string, write_stl_binary_bytes, write_xb_from_brep,
    write_xt_from_brep, xb_to_xt, xt_to_xb, MeshBodyInput, StepOptions, StepSchema, StepUnits,
    XtError,
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
        "colored_box.x_b",
        "assembly_two_parts.x_b",
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

#[test]
fn mesh_brep_coplanar_merge_and_xt_roundtrip() {
    // 10 x 20 x 30 mm triangulated box (12 triangles -> should merge to 6 planar faces, 12 edges, 8 vertices)
    let positions = [
        [0.0, 0.0, 0.0],
        [10.0, 0.0, 0.0],
        [10.0, 20.0, 0.0],
        [0.0, 20.0, 0.0],
        [0.0, 0.0, 30.0],
        [10.0, 0.0, 30.0],
        [10.0, 20.0, 30.0],
        [0.0, 20.0, 30.0],
    ];
    let triangles = [
        [0, 2, 1],
        [0, 3, 2], // z=0
        [4, 5, 6],
        [4, 6, 7], // z=30
        [0, 1, 5],
        [0, 5, 4], // y=0
        [1, 2, 6],
        [1, 6, 5], // x=10
        [2, 3, 7],
        [2, 7, 6], // y=20
        [3, 0, 4],
        [3, 4, 7], // x=0
    ];
    let model = meshes_to_brep(
        &[MeshBodyInput {
            name: "Block10x20x30",
            positions_mm: &positions,
            triangles: &triangles,
            appearance: None,
        }],
        true,
    )
    .unwrap();
    assert_eq!(model.faces.len(), 6);
    assert_eq!(model.edges.len(), 12);
    assert_eq!(model.vertices.len(), 8);

    let xb = write_xb_from_brep(&model).unwrap();
    let xt = write_xt_from_brep(&model).unwrap();
    let parsed_b = extract_brep(&parse_bytes(&xb).unwrap());
    let parsed_t = extract_brep(&parse_bytes(&xt).unwrap());
    assert_eq!(parsed_b.faces.len(), 6);
    assert_eq!(parsed_t.faces.len(), 6);
    assert_eq!(parsed_b.bodies[0].name.as_deref(), Some("Block10x20x30"));

    // Transcode xb <-> xt and convert back to STEP
    let xt2 = xb_to_xt(&xb).unwrap();
    let xb2 = xt_to_xb(&xt2).unwrap();
    let step = convert_bytes_to_step(&xb2, &StepOptions::default()).unwrap();
    assert_eq!(step.faces, 6);
    assert!(step.step.contains("MANIFOLD_SOLID_BREP"));
}

#[test]
fn tessellation_and_mass_properties_closed_solids() {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let tests_dir = manifest_dir
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("solidcraft-parasolid")
        .join("tests");

    let cases: [(&str, f64, f64); 6] = [
        ("box.x_b", 1000.0, 1e-6),
        ("colored_box.x_b", 24000.0, 1e-6),
        ("assembly_two_parts.x_b", 2000.0, 1e-6),
        ("cylinder.x_b", std::f64::consts::PI * 100.0 * 20.0, 0.02),
        (
            "cone_frustum.x_b",
            (std::f64::consts::PI * 20.0 / 3.0) * (100.0 + 40.0 + 16.0),
            0.02,
        ),
        (
            "plate_with_hole.x_b",
            40.0 * 40.0 * 10.0 - std::f64::consts::PI * 25.0 * 10.0,
            0.02,
        ),
    ];

    for (name, exp_vol, tol) in cases {
        let bytes = std::fs::read(tests_dir.join(name)).unwrap();
        let parsed = parse_bytes(&bytes).unwrap();
        let model = extract_brep(&parsed);
        let mesh = tessellate_brep(&model, 48);
        let props = compute_mesh_mass_props(&mesh.mass_positions_mm, &mesh.mass_triangles, 1.0)
            .unwrap_or_else(|| panic!("{name} produced empty mesh"));
        assert!(props.is_closed, "{name} mesh was not closed/watertight");
        let rel = (props.volume - exp_vol).abs() / exp_vol;
        assert!(
            rel < tol,
            "{name} volume {} differed from expected {} (rel={rel})",
            props.volume,
            exp_vol
        );
        let stl = write_stl_binary_bytes(&mesh);
        assert!(stl.len() > 84);
        let obj = write_obj_string(&mesh);
        assert!(obj.contains("\nf "));
    }

    // Also test sphere
    let sph_bytes = std::fs::read(tests_dir.join("sphere.x_b")).unwrap();
    let sph_mesh = tessellate_brep(&extract_brep(&parse_bytes(&sph_bytes).unwrap()), 48);
    let sph_props =
        compute_mesh_mass_props(&sph_mesh.mass_positions_mm, &sph_mesh.mass_triangles, 1.0)
            .unwrap();
    assert!(sph_props.is_closed, "sphere.x_b not closed");
    let sph_exp = (4.0 / 3.0) * std::f64::consts::PI * 1000.0;
    assert!((sph_props.volume - sph_exp).abs() / sph_exp < 0.05);
}
