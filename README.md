<div align="center">

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="assets/logo-dark.svg">
  <source media="(prefers-color-scheme: light)" srcset="assets/logo-light.svg">
  <img alt="solidcraft-parasolid logo" src="assets/logo-dark.svg" width="460">
</picture>

<p>
  <strong>Clean-room Parasolid XT (<code>.x_b</code>, <code>.x_t</code>, <code>.x_p</code>) &amp; Siemens NX (<code>.prt</code>) B-rep importer &amp; exporter plugins for SolidCraft + live in-browser 3D viewer &amp; mass calculator — written in pure Rust and compiled to WebAssembly.</strong>
</p>

<p>
  <a href="https://41opty41.github.io/solidcraft-parasolid/"><strong>Live Browser 3D Viewer &amp; Mass Calculator</strong></a> ·
  <a href="README.md">English</a> ·
  <a href="docs/i18n/README.ru.md">Русский</a> ·
  <a href="docs/i18n/README.zh-CN.md">简体中文</a> ·
  <a href="docs/i18n/README.de.md">Deutsch</a> ·
  <a href="docs/i18n/README.ja.md">日本語</a> ·
  <a href="docs/i18n/README.es.md">Español</a>
</p>

<p>
  <a href="https://github.com/41Opty41/solidcraft-parasolid/actions/workflows/ci.yml"><img src="https://github.com/41Opty41/solidcraft-parasolid/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
  <a href="LICENSE-MIT"><img src="https://img.shields.io/badge/license-MIT%20%2F%20Apache--2.0-blue.svg" alt="License: MIT OR Apache-2.0"></a>
  <img src="https://img.shields.io/badge/SolidCraft-v0.50.0%2B-0891B2.svg" alt="SolidCraft v0.50.0+">
  <img src="https://img.shields.io/badge/Rust-WASM%20Component-orange.svg" alt="Rust WASM Component">
  <a href="DONATE.md"><img src="https://img.shields.io/badge/Donate-BTC%20%7C%20YooMoney-f59e0b.svg" alt="Donate"></a>
</p>

<img src="assets/social-preview.png" alt="solidcraft-parasolid overview" width="760">

</div>

---

## Live Browser Viewer & Mass Calculator

Open any `.x_b`, `.x_t`, `.x_p`, or Siemens NX `.prt` (`SPLMSSTR` & legacy `OLE2/CFB`) file directly in your browser (100% client-side in WebAssembly, zero server uploads):

**https://41opty41.github.io/solidcraft-parasolid/**

- WebGL2 3D B-rep viewer with per-face/body colors, assembly instance placement, and edge wireframes
- Real-time mass, volume, surface area, center of mass, bounding box, and principal moments of inertia powered by pure-Rust [brepmass](https://github.com/41Opty41/brepmass) algorithms
- One-click client-side export to **STEP (`AP214` / `AP242`)**, **Binary STL**, **OBJ**, **`.x_t`**, and **`.x_b`**

---

## Why `solidcraft-parasolid`?

[SolidCraft](https://github.com/tangency-labs/solidcraft) is a pure-Rust parametric 3D CAD application whose core stays strictly open-formats-only (`STEP`, `IGES`, `3MF`). Commercial CAD transmit files such as **Parasolid XT** (`.x_b`, `.x_t`, `.x_p`) are supported through SolidCraft's sandboxed **WebAssembly importer & exporter plugins**.

This repository ships two companion SolidCraft `craft:plugin@0.2.0` WebAssembly Components powered by a pure-Rust port of [xtopen](https://github.com/41Opty41/xtopen):

1. **`solidcraft-parasolid` (`v0.3.0` — Importer)** — opens `.x_b`, `.x_t`, `.x_p`, `.xmt_txt`, `.xmt_bin`, and **Siemens NX `.prt`** (`SPLMSSTR` zlib-compressed & legacy `OLE2/CFB` containers) directly via **File › Open** in SolidCraft `v0.50.0+` (underscore extensions enabled upstream in [tangency-labs/solidcraft#22054](https://github.com/tangency-labs/solidcraft/pull/22054)). Preserves **assemblies (`ASSEMBLY` / `INSTANCE` / `TRANSFORM`)**, **names (`SDL/TYSA_NAME`)**, and **face/body colors & transparency (`SDL/TYSA_COLOUR`)**.
2. **`solidcraft-parasolid-export` (`v0.1.0` — Exporter)** — adds **File › Export › Parasolid Binary (`.x_b`)** and **Parasolid Text (`.x_t`)** slots, including **coplanar face merging** so planar CAD solids export with clean minimal faces and straight edges.

---

## Install in SolidCraft (`v0.50.0+`)

1. From the [Latest Release](https://github.com/41Opty41/solidcraft-parasolid/releases/latest), download:
   - **`solidcraft-parasolid-0.3.0.zip`** (Parasolid XT & NX `.prt` **Importer**)
   - **`solidcraft-parasolid-export-0.1.0.zip`** (Parasolid XT **Exporter**, optional)
2. In SolidCraft, open **Preferences › Plugins › Install plugin…** and select each `.zip` archive (or install from the community catalog).
3. Use **File › Open** for `.x_b`/`.x_t`/`.x_p`/`.prt` models and **File › Export** to write `.x_b` / `.x_t`.

---

## Verification Summary (SolidCraft `v0.50.0`)

- `cargo test --workspace`: all unit & integration tests pass (XT parsing, NX `.prt` SPLMSSTR/OLE2 extraction, STEP export, assemblies/colors, coplanar mesh-to-Brep roundtrip, watertight tessellation & exact divergence-theorem mass properties).
- `solidcraft-cli plugin check` (`0.50.0`): **`0` errors, `0` warnings** on both `solidcraft-parasolid` (importer) and `solidcraft-parasolid-export` (exporter).
- `solidcraft-cli plugin test --require-cases` (`0.50.0`):
  - **Importer (`solidcraft-parasolid 0.3.0`)**: **13/13** cases passed (`box`, `box.x_t`, `cylinder`, `cone_frustum`, `plate_with_hole`, `nx_plate.prt`, `sphere`, `two_boxes`, `nurbs_sheet`, `colored_box`, `assembly_two_parts`, `corrupt`, `empty`).
  - **Exporter (`solidcraft-parasolid-export 0.1.0`)**: **3/3** cases passed (`box_export_xb`, `cylinder_export_xb`, `plate_export_xt`), plus OCCT roundtrip verification.

---

## Native CLI (`xtopen-cli`) & Build from Source

```sh
# Run unit & integration tests
cargo test --workspace

# Convert .prt / .x_b / .x_t directly to STEP on the command line
cargo run --release -p xtopen-cli -- convert model.prt model.step --units mm --schema AP214

# Extract embedded .x_b streams & preview image from an NX .prt file
cargo run --release -p xtopen-cli -- extract-prt model.prt out_dir/

# Transcode binary <-> text Parasolid XT
cargo run --release -p xtopen-cli -- transcode model.x_b model.x_t

# Inspect topology & benchmark
cargo run --release -p xtopen-cli -- inspect model.x_b
cargo run --release -p xtopen-cli -- bench model.x_b --iters 50
```

---

## Architecture

- [`crates/xtopen-core`](crates/xtopen-core/src/lib.rs) — zero-dependency pure-Rust XT parser (`xb.rs`, `xt.rs`), XT writer (`xt_writer.rs`), schema (`schema.rs`), B-rep extractor (`model.rs`), STEP AP214/AP242 writer (`step_writer.rs`), mesh-to-Brep converter (`mesh_brep.rs`), tessellator (`tessellate.rs`), and mass/inertia calculator (`mass.rs`).
- [`crates/xtopen-cli`](crates/xtopen-cli/src/main.rs) — standalone native CLI (`convert`, `transcode`, `inspect`, `bench`).
- [`crates/solidcraft-parasolid-wasm`](crates/solidcraft-parasolid-wasm/src/lib.rs) — `craft:plugin@0.2.0` (`importer-plugin`) WebAssembly Component.
- [`crates/solidcraft-parasolid-export-wasm`](crates/solidcraft-parasolid-export-wasm/src/lib.rs) — `craft:plugin@0.2.0` (`exporter-plugin`) WebAssembly Component.
- [`crates/xtopen-web-wasm`](crates/xtopen-web-wasm/src/lib.rs) & [`docs/`](docs/index.html) — zero-dependency browser WebGL2 viewer & mass calculator.

## Support & License

Dual-licensed under **MIT** ([`LICENSE-MIT`](LICENSE-MIT)) or **Apache-2.0** ([`LICENSE-APACHE`](LICENSE-APACHE)).

If this project saved you time, voluntary support is welcome via Bitcoin or YooMoney — see [`DONATE.md`](DONATE.md).
