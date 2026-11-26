# Changelog

All notable changes to `solidcraft-parasolid` are documented in this file.

## [0.1.0] - 2026-11-26

### Added
- `xtopen-core`: clean-room zero-dependency Rust parser for Form-1/Form-2 binary (`.x_b`, `.x_p`) and text (`.x_t`) Parasolid XT streams, B-rep model extractor, and STEP AP214/AP242 writer.
- `xtopen-cli`: native CLI with `convert`, `inspect`, and `bench` commands.
- `solidcraft-parasolid`: `craft:plugin@0.2.0` WebAssembly component (`wasm32-unknown-unknown`, zero WASI imports) claiming `.x_b`, `.x_t`, `.x_p`, `.xmt_txt`, `.xmt_bin`, `.xb`, and `.xt` in SolidCraft `v0.50.0+`.
- End-to-end verification suite with 10 synthetic test fixtures checked by `solidcraft-cli plugin test` and cross-checked against OCCT.
