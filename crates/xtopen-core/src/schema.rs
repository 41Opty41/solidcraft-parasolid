//! Self-contained Parasolid XT base schema derived from the public XT Format Reference.

use std::collections::BTreeMap;
use std::sync::OnceLock;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FieldSpec {
    pub name: String,
    pub code: char,
    pub ptr_class: i32,
    pub elements: usize,
    pub transmitted: bool,
    pub rec: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NodeSpec {
    pub type_id: i32,
    pub name: String,
    pub transmitted: bool,
    pub variable: bool,
    pub fields: Vec<FieldSpec>,
}

fn f(name: &str, code: char) -> FieldSpec {
    FieldSpec {
        name: name.to_string(),
        code,
        ptr_class: 0,
        elements: 0,
        transmitted: true,
        rec: false,
    }
}

fn fp(name: &str, code: char, ptr_class: i32, elements: usize) -> FieldSpec {
    FieldSpec {
        name: name.to_string(),
        code,
        ptr_class,
        elements,
        transmitted: true,
        rec: false,
    }
}

fn node(type_id: i32, name: &str, fields: Vec<FieldSpec>, variable: bool) -> NodeSpec {
    NodeSpec {
        type_id,
        name: name.to_string(),
        transmitted: true,
        variable,
        fields,
    }
}

fn any_curve_fields() -> Vec<FieldSpec> {
    vec![
        f("node_id", 'd'),
        fp("attributes_groups", 'p', 1019, 0),
        fp("owner", 'p', 1010, 0),
        fp("next", 'p', 1008, 0),
        fp("previous", 'p', 1008, 0),
        fp("geometric_owner", 'p', 141, 0),
        f("sense", 'c'),
    ]
}

fn any_surf_fields() -> Vec<FieldSpec> {
    vec![
        f("node_id", 'd'),
        fp("attributes_groups", 'p', 1019, 0),
        fp("owner", 'p', 1007, 0),
        fp("next", 'p', 1006, 0),
        fp("previous", 'p', 1006, 0),
        fp("geometric_owner", 'p', 141, 0),
        f("sense", 'c'),
    ]
}

fn build_builtin_schema() -> BTreeMap<i32, NodeSpec> {
    let mut nodes = vec![
        node(
            4,
            "SCHEMA",
            vec![
                f("node_id", 'd'),
                f("preceding", 'd'),
                f("first_following", 'd'),
                f("next_sibling", 'd'),
                fp("lattice", 'p', 0, 2),
                fp("mesh", 'f', 0, 2),
                fp("polyline", 'p', 0, 2),
                f("n_new_nodes", 'd'),
                f("n_del_nodes", 'd'),
                f("n_copy_mod_nodes", 'd'),
                f("delta_key", 'd'),
                f("delta_is_forward", 'l'),
                f("id", 'd'),
                f("index_map_offset", 'd'),
            ],
            false,
        ),
        node(
            10,
            "ASSEMBLY",
            vec![
                f("highest_node_id", 'd'),
                fp("attributes_groups", 'p', 1019, 0),
                fp("attribute_chains", 'p', 70, 0),
                fp("list", 'p', 70, 0),
                fp("surface", 'p', 1006, 0),
                fp("curve", 'p', 1008, 0),
                fp("point", 'p', 29, 0),
                fp("key", 'p', 102, 0),
                f("res_size", 'f'),
                f("res_linear", 'f'),
                fp("ref_instance", 'p', 11, 0),
                fp("next", 'p', 10, 0),
                fp("previous", 'p', 10, 0),
                f("state", 'u'),
                fp("owner", 'p', 101, 0),
                f("type", 'u'),
                fp("sub_instance", 'p', 11, 0),
            ],
            false,
        ),
        node(
            11,
            "INSTANCE",
            vec![
                f("node_id", 'd'),
                fp("attributes_groups", 'p', 1019, 0),
                f("type", 'u'),
                fp("part", 'p', 1005, 0),
                fp("transform", 'p', 100, 0),
                fp("assembly", 'p', 10, 0),
                fp("next_in_part", 'p', 11, 0),
                fp("prev_in_part", 'p', 11, 0),
                fp("next_of_part", 'p', 11, 0),
                fp("prev_of_part", 'p', 11, 0),
            ],
            false,
        ),
        node(
            12,
            "BODY",
            vec![
                f("highest_node_id", 'd'),
                fp("attributes_groups", 'p', 1019, 0),
                fp("attribute_chains", 'p', 70, 0),
                fp("surface", 'p', 1006, 0),
                fp("curve", 'p', 1008, 0),
                fp("point", 'p', 29, 0),
                fp("key", 'p', 102, 0),
                f("res_size", 'f'),
                f("res_linear", 'f'),
                fp("ref_instance", 'p', 11, 0),
                fp("next", 'p', 12, 0),
                fp("previous", 'p', 12, 0),
                f("state", 'u'),
                fp("owner", 'p', 101, 0),
                f("body_type", 'u'),
                f("nom_geom_state", 'u'),
                fp("shell", 'p', 13, 0),
                fp("boundary_surface", 'p', 1006, 0),
                fp("boundary_curve", 'p', 1008, 0),
                fp("boundary_point", 'p', 29, 0),
                fp("region", 'p', 19, 0),
                fp("edge", 'p', 16, 0),
                fp("vertex", 'p', 18, 0),
            ],
            false,
        ),
        node(
            13,
            "SHELL",
            vec![
                f("node_id", 'd'),
                fp("attributes_groups", 'p', 1019, 0),
                fp("body", 'p', 12, 0),
                fp("next", 'p', 13, 0),
                fp("face", 'p', 14, 0),
                fp("edge", 'p', 16, 0),
                fp("vertex", 'p', 18, 0),
                fp("region", 'p', 19, 0),
                fp("front_face", 'p', 14, 0),
            ],
            false,
        ),
        node(
            14,
            "FACE",
            vec![
                f("node_id", 'd'),
                fp("attributes_groups", 'p', 1019, 0),
                f("tolerance", 'f'),
                fp("next", 'p', 14, 0),
                fp("previous", 'p', 14, 0),
                fp("loop", 'p', 15, 0),
                fp("shell", 'p', 13, 0),
                fp("surface", 'p', 1006, 0),
                f("sense", 'c'),
                fp("next_on_surface", 'p', 14, 0),
                fp("previous_on_surface", 'p', 14, 0),
                fp("next_front", 'p', 14, 0),
                fp("previous_front", 'p', 14, 0),
                fp("front_shell", 'p', 13, 0),
            ],
            false,
        ),
        node(
            15,
            "LOOP",
            vec![
                f("node_id", 'd'),
                fp("attributes_groups", 'p', 1019, 0),
                fp("halfedge", 'p', 17, 0),
                fp("face", 'p', 14, 0),
                fp("next", 'p', 15, 0),
            ],
            false,
        ),
        node(
            16,
            "EDGE",
            vec![
                f("node_id", 'd'),
                fp("attributes_groups", 'p', 1019, 0),
                f("tolerance", 'f'),
                fp("halfedge", 'p', 17, 0),
                fp("previous", 'p', 16, 0),
                fp("next", 'p', 16, 0),
                fp("curve", 'p', 1008, 0),
                fp("next_on_curve", 'p', 16, 0),
                fp("previous_on_curve", 'p', 16, 0),
                fp("owner", 'p', 1029, 0),
            ],
            false,
        ),
        node(
            17,
            "HALFEDGE",
            vec![
                fp("attributes_groups", 'p', 1019, 0),
                fp("loop", 'p', 15, 0),
                fp("forward", 'p', 17, 0),
                fp("backward", 'p', 17, 0),
                fp("vertex", 'p', 18, 0),
                fp("other", 'p', 17, 0),
                fp("edge", 'p', 16, 0),
                fp("curve", 'p', 1008, 0),
                fp("next_at_vx", 'p', 17, 0),
                f("sense", 'c'),
            ],
            false,
        ),
        node(
            18,
            "VERTEX",
            vec![
                f("node_id", 'd'),
                fp("attributes_groups", 'p', 1019, 0),
                fp("halfedge", 'p', 17, 0),
                fp("previous", 'p', 18, 0),
                fp("next", 'p', 18, 0),
                fp("point", 'p', 29, 0),
                f("tolerance", 'f'),
                fp("owner", 'p', 1029, 0),
            ],
            false,
        ),
        node(
            19,
            "REGION",
            vec![
                f("node_id", 'd'),
                fp("attributes_groups", 'p', 1019, 0),
                fp("body", 'p', 12, 0),
                fp("next", 'p', 19, 0),
                fp("previous", 'p', 19, 0),
                fp("shell", 'p', 13, 0),
                f("type", 'c'),
            ],
            false,
        ),
        node(
            29,
            "POINT",
            vec![
                f("node_id", 'd'),
                fp("attributes_groups", 'p', 1019, 0),
                fp("owner", 'p', 1011, 0),
                fp("next", 'p', 29, 0),
                fp("previous", 'p', 29, 0),
                f("pvec", 'v'),
            ],
            false,
        ),
        node(
            30,
            "LINE",
            [any_curve_fields(), vec![f("pvec", 'v'), f("direction", 'v')]].concat(),
            false,
        ),
        node(
            31,
            "CIRCLE",
            [
                any_curve_fields(),
                vec![
                    f("centre", 'v'),
                    f("normal", 'v'),
                    f("x_axis", 'v'),
                    f("radius", 'f'),
                ],
            ]
            .concat(),
            false,
        ),
        node(
            32,
            "ELLIPSE",
            [
                any_curve_fields(),
                vec![
                    f("centre", 'v'),
                    f("normal", 'v'),
                    f("x_axis", 'v'),
                    f("major_radius", 'f'),
                    f("minor_radius", 'f'),
                ],
            ]
            .concat(),
            false,
        ),
        node(
            38,
            "INTERSECTION",
            [
                any_curve_fields(),
                vec![
                    fp("surface", 'p', 1006, 2),
                    fp("chart", 'p', 40, 0),
                    fp("start", 'p', 41, 0),
                    fp("end", 'p', 41, 0),
                ],
            ]
            .concat(),
            false,
        ),
        node(
            40,
            "CHART",
            vec![
                f("base_parameter", 'f'),
                f("base_scale", 'f'),
                f("chart_count", 'd'),
                f("chordal_error", 'f'),
                f("angular_error", 'f'),
                fp("parameter_error", 'f', 0, 2),
                fp("hvec", 'h', 0, 1),
            ],
            true,
        ),
        node(
            41,
            "LIMIT",
            vec![f("type", 'c'), fp("hvec", 'h', 0, 1)],
            true,
        ),
        node(45, "BSPLINE_VERTICES", vec![fp("vertices", 'f', 0, 1)], true),
        node(
            50,
            "PLANE",
            [
                any_surf_fields(),
                vec![f("pvec", 'v'), f("normal", 'v'), f("x_axis", 'v')],
            ]
            .concat(),
            false,
        ),
        node(
            51,
            "CYLINDER",
            [
                any_surf_fields(),
                vec![
                    f("pvec", 'v'),
                    f("axis", 'v'),
                    f("radius", 'f'),
                    f("x_axis", 'v'),
                ],
            ]
            .concat(),
            false,
        ),
        node(
            52,
            "CONE",
            [
                any_surf_fields(),
                vec![
                    f("pvec", 'v'),
                    f("axis", 'v'),
                    f("radius", 'f'),
                    f("sin_half_angle", 'f'),
                    f("cos_half_angle", 'f'),
                    f("x_axis", 'v'),
                ],
            ]
            .concat(),
            false,
        ),
        node(
            53,
            "SPHERE",
            [
                any_surf_fields(),
                vec![
                    f("centre", 'v'),
                    f("radius", 'f'),
                    f("axis", 'v'),
                    f("x_axis", 'v'),
                ],
            ]
            .concat(),
            false,
        ),
        node(
            54,
            "TORUS",
            [
                any_surf_fields(),
                vec![
                    f("centre", 'v'),
                    f("axis", 'v'),
                    f("major_radius", 'f'),
                    f("minor_radius", 'f'),
                    f("x_axis", 'v'),
                ],
            ]
            .concat(),
            false,
        ),
        node(
            56,
            "BLENDED_EDGE",
            [
                any_surf_fields(),
                vec![
                    f("blend_type", 'c'),
                    fp("surface", 'p', 1006, 2),
                    fp("spine", 'p', 1008, 0),
                    fp("range", 'f', 0, 2),
                    fp("thumb_weight", 'f', 0, 2),
                    fp("boundary", 'p', 1006, 2),
                    fp("start", 'p', 41, 0),
                    fp("end", 'p', 41, 0),
                ],
            ]
            .concat(),
            false,
        ),
        node(
            59,
            "BLEND_BOUND",
            [
                any_surf_fields(),
                vec![f("boundary", 'n'), fp("blend", 'p', 1006, 0)],
            ]
            .concat(),
            false,
        ),
        node(
            60,
            "OFFSET_SURF",
            [
                any_surf_fields(),
                vec![
                    f("check", 'c'),
                    f("true_offset", 'l'),
                    fp("surface", 'p', 1006, 0),
                    f("offset", 'f'),
                    f("scale", 'f'),
                ],
            ]
            .concat(),
            false,
        ),
        node(
            67,
            "SWEPT_SURF",
            [
                any_surf_fields(),
                vec![
                    fp("section", 'p', 1008, 0),
                    f("sweep", 'v'),
                    f("scale", 'f'),
                ],
            ]
            .concat(),
            false,
        ),
        node(
            68,
            "SPUN_SURF",
            [
                any_surf_fields(),
                vec![
                    fp("profile", 'p', 1008, 0),
                    f("base", 'v'),
                    f("axis", 'v'),
                    f("start", 'v'),
                    f("end", 'v'),
                    f("start_param", 'f'),
                    f("end_param", 'f'),
                    f("x_axis", 'v'),
                    f("scale", 'f'),
                ],
            ]
            .concat(),
            false,
        ),
        node(
            70,
            "LIST",
            vec![
                f("node_id", 'd'),
                fp("owner", 'p', 1013, 0),
                fp("next", 'p', 70, 0),
                fp("previous", 'p', 70, 0),
                f("list_type", 'd'),
                f("list_length", 'd'),
                f("block_length", 'd'),
                f("size_of_entry", 'd'),
                fp("list_block", 'p', 1012, 0),
                fp("finger_block", 'p', 1012, 0),
                f("finger_index", 'd'),
                f("notransmit", 'l'),
            ],
            false,
        ),
        node(
            74,
            "POINTER_LIS_BLOCK",
            vec![
                f("n_entries", 'd'),
                fp("next_block", 'p', 74, 0),
                fp("entries", 'p', 0, 1),
            ],
            true,
        ),
        node(79, "ATT_DEF_ID", vec![fp("string", 'c', 0, 1)], true),
        node(
            80,
            "ATTRIB_DEF",
            vec![
                fp("next", 'p', 80, 0),
                fp("identifier", 'p', 79, 0),
                f("type_id", 'd'),
                fp("actions", 'u', 0, 8),
                fp("field_names", 'p', 99, 0),
                fp("legal_owners", 'l', 0, 14),
                fp("fields", 'u', 0, 1),
            ],
            true,
        ),
        node(
            81,
            "ATTRIBUTE",
            vec![
                f("node_id", 'd'),
                fp("definition", 'p', 80, 0),
                fp("owner", 'p', 1015, 0),
                fp("next", 'p', 1019, 0),
                fp("previous", 'p', 1019, 0),
                fp("next_of_type", 'p', 81, 0),
                fp("previous_of_type", 'p', 81, 0),
                fp("fields", 'p', 1018, 1),
            ],
            true,
        ),
        node(82, "INT_VALUES", vec![fp("values", 'd', 0, 1)], true),
        node(83, "REAL_VALUES", vec![fp("values", 'f', 0, 1)], true),
        node(84, "CHAR_VALUES", vec![fp("values", 'c', 0, 1)], true),
        node(85, "POINT_VALUES", vec![fp("values", 'v', 0, 1)], true),
        node(86, "VECTOR_VALUES", vec![fp("values", 'v', 0, 1)], true),
        node(87, "AXIS_VALUES", vec![fp("values", 'v', 0, 1)], true),
        node(88, "TAG_VALUES", vec![fp("values", 't', 0, 1)], true),
        node(89, "DIRECTION_VALUES", vec![fp("values", 'v', 0, 1)], true),
        node(
            90,
            "GROUP",
            vec![
                f("node_id", 'd'),
                fp("attributes_groups", 'p', 1019, 0),
                fp("owner", 'p', 1005, 0),
                fp("next", 'p', 1019, 0),
                fp("previous", 'p', 1019, 0),
                f("type", 'u'),
                fp("first_member", 'p', 91, 0),
            ],
            false,
        ),
        node(
            91,
            "MEMBER_OF_GROUP",
            vec![
                f("dummy_node_id", 'd'),
                fp("owning_group", 'p', 90, 0),
                fp("owner", 'p', 1017, 0),
                fp("next", 'p', 1019, 0),
                fp("previous", 'p', 1019, 0),
                fp("next_member", 'p', 91, 0),
                fp("previous_member", 'p', 91, 0),
            ],
            false,
        ),
        node(98, "UNICODE_VALUES", vec![fp("values", 'w', 0, 1)], true),
        node(99, "FIELD_NAMES", vec![fp("names", 'p', 1037, 1)], true),
        node(
            100,
            "TRANSFORM",
            vec![
                f("node_id", 'd'),
                fp("owner", 'p', 1023, 0),
                fp("next", 'p', 100, 0),
                fp("previous", 'p', 100, 0),
                fp("rotation_matrix", 'f', 0, 9),
                f("translation_vector", 'v'),
                f("scale", 'f'),
                f("flag", 'd'),
                f("perspective_vector", 'v'),
            ],
            false,
        ),
        node(
            101,
            "WORLD",
            vec![
                fp("assembly", 'p', 10, 0),
                fp("attribute", 'p', 81, 0),
                fp("body", 'p', 12, 0),
                fp("transform", 'p', 100, 0),
                fp("surface", 'p', 1006, 0),
                fp("curve", 'p', 1008, 0),
                fp("point", 'p', 29, 0),
                f("alive", 'l'),
                fp("attrib_def", 'p', 80, 0),
                f("highest_id", 'd'),
                f("current_id", 'd'),
            ],
            false,
        ),
        node(102, "KEY", vec![fp("string", 'c', 0, 1)], true),
        node(
            120,
            "PE_SURF",
            [
                any_surf_fields(),
                vec![
                    f("type", 'c'),
                    fp("data", 'p', 1027, 0),
                    fp("tf", 'p', 100, 0),
                    fp("internal_geom", 'p', 1028, 1),
                ],
            ]
            .concat(),
            true,
        ),
        node(
            121,
            "INT_PE_DATA",
            vec![
                f("geom_type", 'd'),
                fp("real_array", 'p', 83, 0),
                fp("int_array", 'p', 82, 0),
            ],
            false,
        ),
        node(
            122,
            "EXT_PE_DATA",
            vec![
                fp("key", 'p', 102, 0),
                fp("real_array", 'p', 83, 0),
                fp("int_array", 'p', 82, 0),
                fp("data", 'f', 0, 1),
            ],
            true,
        ),
        node(
            124,
            "B_SURFACE",
            [
                any_surf_fields(),
                vec![fp("nurbs", 'p', 126, 0), fp("data", 'p', 125, 0)],
            ]
            .concat(),
            false,
        ),
        node(
            125,
            "SURFACE_DATA",
            vec![
                f("original_uint", 'i'),
                f("original_vint", 'i'),
                f("extended_uint", 'i'),
                f("extended_vint", 'i'),
                f("self_int", 'u'),
                f("original_u_start", 'c'),
                f("original_u_end", 'c'),
                f("original_v_start", 'c'),
                f("original_v_end", 'c'),
                f("extended_u_start", 'c'),
                f("extended_u_end", 'c'),
                f("extended_v_start", 'c'),
                f("extended_v_end", 'c'),
                f("analytic_form_type", 'c'),
                f("swept_form_type", 'c'),
                f("spun_form_type", 'c'),
                f("blend_form_type", 'c'),
                fp("analytic_form", 'p', 0, 0),
                fp("swept_form", 'p', 0, 0),
                fp("spun_form", 'p', 0, 0),
                fp("blend_form", 'p', 0, 0),
            ],
            false,
        ),
        node(
            126,
            "NURBS_SURF",
            vec![
                f("u_periodic", 'l'),
                f("v_periodic", 'l'),
                f("u_degree", 'n'),
                f("v_degree", 'n'),
                f("n_u_vertices", 'd'),
                f("n_v_vertices", 'd'),
                f("u_knot_type", 'u'),
                f("v_knot_type", 'u'),
                f("n_u_knots", 'd'),
                f("n_v_knots", 'd'),
                f("rational", 'l'),
                f("u_closed", 'l'),
                f("v_closed", 'l'),
                f("surface_form", 'u'),
                f("vertex_dim", 'n'),
                fp("bspline_vertices", 'p', 45, 0),
                fp("u_knot_mult", 'p', 127, 0),
                fp("v_knot_mult", 'p', 127, 0),
                fp("u_knots", 'p', 128, 0),
                fp("v_knots", 'p', 128, 0),
            ],
            false,
        ),
        node(127, "KNOT_MULT", vec![fp("mult", 'n', 0, 1)], true),
        node(128, "KNOT_SET", vec![fp("knots", 'f', 0, 1)], true),
        node(
            130,
            "PE_CURVE",
            [
                any_curve_fields(),
                vec![
                    f("type", 'c'),
                    fp("data", 'p', 1027, 0),
                    fp("tf", 'p', 100, 0),
                    fp("internal_geom", 'p', 1028, 1),
                ],
            ]
            .concat(),
            true,
        ),
        node(
            133,
            "TRIMMED_CURVE",
            [
                any_curve_fields(),
                vec![
                    fp("basis_curve", 'p', 1008, 0),
                    f("point_1", 'v'),
                    f("point_2", 'v'),
                    f("parm_1", 'f'),
                    f("parm_2", 'f'),
                ],
            ]
            .concat(),
            false,
        ),
        node(
            134,
            "B_CURVE",
            [
                any_curve_fields(),
                vec![fp("nurbs", 'p', 136, 0), fp("data", 'p', 135, 0)],
            ]
            .concat(),
            false,
        ),
        node(
            135,
            "CURVE_DATA",
            vec![f("self_int", 'u'), fp("analytic_form", 'p', 0, 0)],
            false,
        ),
        node(
            136,
            "NURBS_CURVE",
            vec![
                f("degree", 'n'),
                f("n_vertices", 'd'),
                f("vertex_dim", 'n'),
                f("n_knots", 'd'),
                f("knot_type", 'u'),
                f("periodic", 'l'),
                f("closed", 'l'),
                f("rational", 'l'),
                f("curve_form", 'u'),
                fp("bspline_vertices", 'p', 45, 0),
                fp("knot_mult", 'p', 127, 0),
                fp("knots", 'p', 128, 0),
            ],
            false,
        ),
        node(
            137,
            "SP_CURVE",
            [
                any_curve_fields(),
                vec![
                    fp("surface", 'p', 1006, 0),
                    fp("b_curve", 'p', 134, 0),
                    fp("original", 'p', 1008, 0),
                    f("tolerance_to_original", 'f'),
                ],
            ]
            .concat(),
            false,
        ),
        node(
            141,
            "GEOMETRIC_OWNER",
            vec![
                fp("owner", 'p', 1003, 0),
                fp("next", 'p', 141, 0),
                fp("previous", 'p', 141, 0),
                fp("shared_geometry", 'p', 1003, 0),
            ],
            false,
        ),
        node(
            163,
            "HELIX_CU_FORM",
            vec![
                f("axis_pt", 'v'),
                f("axis_dir", 'v'),
                f("hand", 'c'),
                f("turns", 'i'),
                f("pitch", 'f'),
                f("gap", 'f'),
                f("tol", 'f'),
            ],
            false,
        ),
        node(
            184,
            "HELIX_SU_FORM",
            vec![
                f("axis_pt", 'v'),
                f("axis_dir", 'v'),
                f("point", 'v'),
                f("hand", 'c'),
                f("turns", 'i'),
                f("pitch", 'f'),
                f("tol", 'f'),
            ],
            false,
        ),
    ];

    let mut map = BTreeMap::new();
    for n in nodes.drain(..) {
        map.insert(n.type_id, n);
    }
    map
}

static BUILTIN_SCHEMA: OnceLock<BTreeMap<i32, NodeSpec>> = OnceLock::new();

pub fn builtin_schema() -> &'static BTreeMap<i32, NodeSpec> {
    BUILTIN_SCHEMA.get_or_init(build_builtin_schema)
}

pub fn effective(node: &NodeSpec) -> Vec<FieldSpec> {
    node.fields
        .iter()
        .filter(|f| f.transmitted || f.elements == 1)
        .cloned()
        .collect()
}
