"""Generate clean-room synthetic Parasolid XT fixtures and SolidCraft plugin test cases."""

from __future__ import annotations

import json
import math
import os
from pathlib import Path
import sys

_env_xtopen = os.environ.get("XTOPEN_ROOT")
if _env_xtopen:
    _xr = Path(_env_xtopen)
    sys.path.insert(0, str(_xr / "src"))
    sys.path.insert(0, str(_xr))
else:
    _sib = Path(__file__).resolve().parent.parent.parent / "xtopen"
    if _sib.exists():
        sys.path.insert(0, str(_sib / "src"))
        sys.path.insert(0, str(_sib))

from tests.conftest import (
    make_box,
    make_cone_frustum,
    make_cylinder,
    make_plate_with_hole,
    make_rational_nurbs_patch,
    make_sphere_shell,
    make_two_boxes,
)
from xtopen import write_xb, write_xt


def main() -> None:
    out_dir = Path(__file__).resolve().parent.parent / "solidcraft-parasolid" / "tests"
    out_dir.mkdir(parents=True, exist_ok=True)
    for old in out_dir.iterdir():
        if old.is_file():
            old.unlink()

    # Parasolid XT coordinates are in meters; 0.02 x 0.01 x 0.005 m = 20 x 10 x 5 mm (1000 mm^3).
    # SolidCraft v0.50.0+ natively supports underscore extensions (.x_b, .x_t, .x_p, .xmt_txt, .xmt_bin).
    fixtures = [
        (
            "box.x_b",
            write_xb(make_box(0.02, 0.01, 0.005)),
            {
                "entry": "import",
                "file": "box.x_b",
                "expect": {
                    "reply": {"bodies": 1},
                    "model": {"body_count": 1, "total": {"volume_mm3": 1000.0}},
                    "tolerance": 1e-4,
                },
            },
        ),
        (
            "box.x_t",
            write_xt(make_box(0.02, 0.01, 0.005)),
            {
                "entry": "import",
                "file": "box.x_t",
                "expect": {
                    "reply": {"bodies": 1},
                    "model": {"body_count": 1, "total": {"volume_mm3": 1000.0}},
                    "tolerance": 1e-4,
                },
            },
        ),
        (
            "cylinder.x_b",
            write_xb(make_cylinder(0.01, 0.02)),
            {
                "entry": "import",
                "file": "cylinder.x_b",
                "expect": {
                    "reply": {"bodies": 1},
                    "model": {"body_count": 1, "total": {"volume_mm3": math.pi * 100.0 * 20.0}},
                    "tolerance": 1e-3,
                },
            },
        ),
        (
            "cone_frustum.x_b",
            write_xb(make_cone_frustum(0.01, 0.004, 0.02)),
            {
                "entry": "import",
                "file": "cone_frustum.x_b",
                "expect": {
                    "reply": {"bodies": 1},
                    "model": {
                        "body_count": 1,
                        "total": {"volume_mm3": (math.pi * 20.0 / 3.0) * (100.0 + 40.0 + 16.0)},
                    },
                    "tolerance": 1e-3,
                },
            },
        ),
        (
            "plate_with_hole.x_b",
            write_xb(make_plate_with_hole(0.04, 0.04, 0.01, 0.005)),
            {
                "entry": "import",
                "file": "plate_with_hole.x_b",
                "expect": {
                    "reply": {"bodies": 1},
                    "model": {"body_count": 1, "total": {"volume_mm3": (1600.0 - math.pi * 25.0) * 10.0}},
                    "tolerance": 1e-3,
                },
            },
        ),
        (
            "sphere.x_b",
            write_xb(make_sphere_shell(0.01)),
            {
                "entry": "import",
                "file": "sphere.x_b",
                "expect": {
                    "reply": {"bodies": 1},
                    "model": {"body_count": 1, "total": {"volume_mm3": (4.0 / 3.0) * math.pi * 1000.0}},
                    "tolerance": 1e-3,
                },
            },
        ),
        (
            "two_boxes.x_b",
            write_xb(make_two_boxes()),
            {
                "entry": "import",
                "file": "two_boxes.x_b",
                "expect": {
                    "reply": {"bodies": 2},
                    "model": {"body_count": 2, "total": {"volume_mm3": 3e9}},
                    "tolerance": 1e-4,
                },
            },
        ),
        (
            "nurbs_sheet.x_b",
            write_xb(make_rational_nurbs_patch()),
            {
                "entry": "import",
                "file": "nurbs_sheet.x_b",
                "expect": {
                    "reply": {"bodies": 1},
                },
            },
        ),
        (
            "corrupt.x_b",
            b"NOT_A_PARASOLID_FILE_1234567890",
            {
                "entry": "import",
                "file": "corrupt.x_b",
                "expect": {"error": "invalid_input"},
            },
        ),
        (
            "empty.x_b",
            b"",
            {
                "entry": "import",
                "file": "empty.x_b",
                "expect": {"error": "invalid_input"},
            },
        ),
    ]

    for fname, raw, case in fixtures:
        fpath = out_dir / fname
        if isinstance(raw, str):
            fpath.write_text(raw, encoding="latin-1")
        else:
            fpath.write_bytes(raw)
        cpath = out_dir / f"{fname.replace('.', '_')}.json"
        cpath.write_text(json.dumps(case, indent=2) + "\n", encoding="utf-8")

    print(f"Generated {len(fixtures)} test fixtures in {out_dir}")


if __name__ == "__main__":
    main()
