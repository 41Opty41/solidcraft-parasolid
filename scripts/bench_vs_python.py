"""Benchmark Python xtopen vs Native Rust xtopen-cli vs SolidCraft WASM plugin + OCCT cross-check."""

from __future__ import annotations

import json
import math
import os
from pathlib import Path
import shutil
import statistics
import subprocess
import sys
import tempfile
import time

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
    make_ellipse_and_intersection_sheets,
    make_plate_with_hole,
    make_rational_nurbs_patch,
    make_sphere_shell,
    make_torus_sheet,
    make_two_boxes,
)
from xtopen import dumps_step, extract_brep, parse_xb, parse_xt, write_xb, write_xt
from xtopen.schema import builtin_schema, effective
from xtopen.xb import ParseResult, XtNode


def _as_bytes(raw: bytes | str) -> bytes:
    return raw.encode("latin-1") if isinstance(raw, str) else raw


def make_grid_boxes(nx: int, ny: int) -> ParseResult:
    """Create nx * ny disjoint solid boxes in one XT stream (clean synthetic stress test)."""
    base_box = make_box(0.01, 0.01, 0.01)
    schema = builtin_schema()
    fmeta_by_type = {
        tid: {f["name"]: f for f in effective(snode)}
        for tid, snode in schema.items()
    }
    max_idx = max(n.index for n in base_box.nodes) + 10
    all_nodes: list[XtNode] = []
    for k in range(nx * ny):
        ix, iy = divmod(k, ny)
        dx, dy = ix * 0.02, iy * 0.02
        offset = k * max_idx
        for n in base_box.nodes:
            fmeta = fmeta_by_type[n.type]
            new_fields = []
            for fname, val in n.fields:
                meta = fmeta.get(fname)
                if meta and meta["code"] == "p":
                    nv = [x + offset if isinstance(x, int) and x > 0 else x for x in val]
                    new_fields.append((fname, nv))
                elif fname == "pvec" and n.type in (29, 30, 50):
                    new_fields.append((fname, [[p[0] + dx, p[1] + dy, p[2]] for p in val]))
                else:
                    new_fields.append((fname, val))
            all_nodes.append(
                XtNode(
                    type=n.type,
                    name=n.name,
                    index=n.index + offset,
                    fields=new_fields,
                    varlen=n.varlen,
                )
            )
    return ParseResult(header=base_box.header, nodes=all_nodes, remaining_bytes=0)


def bench_python(raw: bytes, is_text: bool, name: str, iters: int) -> dict:
    parsed0 = parse_xt(raw) if is_text else parse_xb(raw)
    model0 = extract_brep(parsed0)
    step0 = dumps_step(model0, units="mm", name=name)

    t_parse, t_brep, t_step, t_total = [], [], [], []
    for _ in range(iters):
        t0 = time.perf_counter()
        parsed = parse_xt(raw) if is_text else parse_xb(raw)
        t1 = time.perf_counter()
        model = extract_brep(parsed)
        t2 = time.perf_counter()
        step = dumps_step(model, units="mm", name=name)
        t3 = time.perf_counter()
        t_parse.append((t1 - t0) * 1000.0)
        t_brep.append((t2 - t1) * 1000.0)
        t_step.append((t3 - t2) * 1000.0)
        t_total.append((t3 - t0) * 1000.0)

    return {
        "step_bytes": len(step0),
        "median_ms": {
            "parse": statistics.median(t_parse),
            "brep": statistics.median(t_brep),
            "step": statistics.median(t_step),
            "total": statistics.median(t_total),
        },
        "mean_ms": {
            "parse": statistics.mean(t_parse),
            "brep": statistics.mean(t_brep),
            "step": statistics.mean(t_step),
            "total": statistics.mean(t_total),
        },
    }


def bench_wasm_solidcraft(solidcraft_cli: Path, plugin_dir: Path, fname: str, raw: bytes) -> float | None:
    if not solidcraft_cli.exists():
        return None
    with tempfile.TemporaryDirectory() as td:
        pkg = Path(td) / "solidcraft-parasolid"
        tests_dir = pkg / "tests"
        tests_dir.mkdir(parents=True)
        shutil.copy2(plugin_dir / "plugin.json", pkg / "plugin.json")
        shutil.copy2(plugin_dir / "solidcraft-parasolid.wasm", pkg / "solidcraft-parasolid.wasm")
        (tests_dir / fname).write_bytes(raw)
        (tests_dir / "bench_case.json").write_text(
            json.dumps({"entry": "import", "file": fname, "expect": {}}),
            encoding="utf-8",
        )
        proc = subprocess.run(
            [str(solidcraft_cli), "plugin", "test", str(pkg), "--json"],
            capture_output=True,
            text=True,
            check=True,
        )
        data = json.loads(proc.stdout)
        cases = data.get("cases") or []
        if cases and "elapsed_ms" in cases[0]:
            return float(cases[0]["elapsed_ms"])
        if cases and "ms" in cases[0]:
            return float(cases[0]["ms"])
        return None


def verify_occt_parity(rust_cli: Path) -> list[dict]:
    from OCP.BRepCheck import BRepCheck_Analyzer
    from OCP.BRepGProp import BRepGProp
    from OCP.GProp import GProp_GProps
    from OCP.IFSelect import IFSelect_RetDone
    from OCP.STEPControl import STEPControl_Reader

    def inspect_step(step_path: Path) -> tuple[bool, float, float]:
        reader = STEPControl_Reader()
        status = reader.ReadFile(str(step_path))
        if status != IFSelect_RetDone:
            return False, 0.0, 0.0
        reader.TransferRoots()
        shape = reader.OneShape()
        valid = bool(BRepCheck_Analyzer(shape).IsValid())
        vp = GProp_GProps()
        sp = GProp_GProps()
        BRepGProp.VolumeProperties_s(shape, vp)
        BRepGProp.SurfaceProperties_s(shape, sp)
        return valid, float(vp.Mass()), float(sp.Mass())

    shapes = [
        ("box", make_box(0.02, 0.01, 0.005)),
        ("cylinder", make_cylinder(0.01, 0.02)),
        ("cone_frustum", make_cone_frustum(0.01, 0.004, 0.02)),
        ("plate_with_hole", make_plate_with_hole(0.04, 0.04, 0.01, 0.005)),
        ("sphere", make_sphere_shell(0.01)),
        ("torus_sheet", make_torus_sheet(0.01, 0.0025)),
        ("nurbs_patch", make_rational_nurbs_patch()),
        ("two_boxes", make_two_boxes()),
    ]
    results = []
    with tempfile.TemporaryDirectory() as td:
        td_path = Path(td)
        for name, pr in shapes:
            xb = td_path / f"{name}.x_b"
            xb.write_bytes(_as_bytes(write_xb(pr)))
            py_stp = td_path / f"{name}_py.step"
            ru_stp = td_path / f"{name}_ru.step"
            py_stp.write_text(dumps_step(pr, units="mm", name=name), encoding="ascii")
            subprocess.run(
                [str(rust_cli), "convert", str(xb), str(ru_stp), "--units", "mm"],
                capture_output=True,
                check=True,
            )
            pv, pvol, parea = inspect_step(py_stp)
            rv, rvol, rarea = inspect_step(ru_stp)
            results.append({
                "shape": name,
                "py_valid": pv,
                "rust_valid": rv,
                "vol_rel_diff": abs(pvol - rvol) / max(1e-9, abs(pvol)),
                "area_rel_diff": abs(parea - rarea) / max(1e-9, abs(parea)),
            })
    return results


def find_solidcraft_cli(root: Path) -> Path:
    tools = root / ".tools"
    for pat in ("solidcraft-0.50.0/**/solidcraft-cli.exe", "**/solidcraft-cli.exe"):
        matches = sorted(tools.glob(pat))
        if matches:
            return matches[-1]
    return tools / "solidcraft" / "solidcraft-0.49.0-windows-x64-portable" / "solidcraft-cli.exe"


def main() -> None:
    root = Path(__file__).resolve().parent.parent
    rust_cli = root / "target" / "release" / "xtopen-cli.exe"
    solidcraft_cli = find_solidcraft_cli(root)
    plugin_dir = root / "solidcraft-parasolid"

    print("=== OCCT Cross-Check (Python STEP vs Rust STEP) ===")
    parity = verify_occt_parity(rust_cli)
    for row in parity:
        assert row["py_valid"] and row["rust_valid"], f"OCCT check failed: {row}"
        assert row["vol_rel_diff"] < 1e-9 and row["area_rel_diff"] < 1e-9, f"Geometry mismatch: {row}"
        print(f"  OK {row['shape']:<26} py_valid={row['py_valid']} rust_valid={row['rust_valid']} dv={row['vol_rel_diff']:.1e} da={row['area_rel_diff']:.1e}")

    cases = [
        ("plate_with_hole (x_b)", False, _as_bytes(write_xb(make_plate_with_hole(0.04, 0.04, 0.01, 0.005))), 100),
        ("plate_with_hole (x_t)", True, _as_bytes(write_xt(make_plate_with_hole(0.04, 0.04, 0.01, 0.005))), 100),
        ("nurbs_patch (x_b)", False, _as_bytes(write_xb(make_rational_nurbs_patch())), 100),
        ("25_boxes_150_faces (x_b)", False, _as_bytes(write_xb(make_grid_boxes(5, 5))), 30),
        ("25_boxes_150_faces (x_t)", True, _as_bytes(write_xt(make_grid_boxes(5, 5))), 30),
        ("100_boxes_600_faces (x_b)", False, _as_bytes(write_xb(make_grid_boxes(10, 10))), 10),
        ("100_boxes_600_faces (x_t)", True, _as_bytes(write_xt(make_grid_boxes(10, 10))), 10),
    ]

    rows = []
    with tempfile.TemporaryDirectory() as td:
        td_path = Path(td)
        for label, is_text, raw, iters in cases:
            ext = ".x_t" if is_text else ".x_b"
            slug = label.replace(" ", "_").replace("(", "").replace(")", "")
            fpath = td_path / f"{slug}{ext}"
            fpath.write_bytes(raw)

            py_res = bench_python(raw, is_text, fpath.stem, iters)
            rust_proc = subprocess.run(
                [str(rust_cli), "bench", str(fpath), "--iters", str(iters)],
                capture_output=True,
                text=True,
                check=True,
            )
            rust_res = json.loads(rust_proc.stdout)
            wasm_e2e_ms = bench_wasm_solidcraft(solidcraft_cli, plugin_dir, fpath.name, raw)

            py_tot = py_res["median_ms"]["total"]
            ru_tot = rust_res["median_ms"]["total"]
            speedup = py_tot / ru_tot if ru_tot > 0 else 0.0
            rows.append({
                "case": label,
                "input_kb": round(len(raw) / 1024.0, 1),
                "iters": iters,
                "py_ms": py_res["median_ms"],
                "rust_ms": rust_res["median_ms"],
                "wasm_solidcraft_e2e_ms": wasm_e2e_ms,
                "speedup_x": round(speedup, 1),
            })

    print("\n=== Benchmark Summary ===")
    print("| Case | Size (KB) | Python (ms) | Rust Native (ms) | Speedup | Parse x | BRep x | STEP x | SolidCraft WASM E2E (ms) |")
    print("|---|---:|---:|---:|---:|---:|---:|---:|---:|")
    for r in rows:
        pm, rm = r["py_ms"], r["rust_ms"]
        px = pm["parse"] / rm["parse"] if rm["parse"] > 0 else 0
        bx = pm["brep"] / rm["brep"] if rm["brep"] > 0 else 0
        sx = pm["step"] / rm["step"] if rm["step"] > 0 else 0
        wm = f"{r['wasm_solidcraft_e2e_ms']:.1f}" if r["wasm_solidcraft_e2e_ms"] is not None else "-"
        print(
            f"| {r['case']} | {r['input_kb']:.1f} | {pm['total']:.3f} | {rm['total']:.3f} | "
            f"**{r['speedup_x']:.1f}x** | {px:.1f}x | {bx:.1f}x | {sx:.1f}x | {wm} |"
        )


if __name__ == "__main__":
    main()
