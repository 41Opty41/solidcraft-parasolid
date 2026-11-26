<div align="center">

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="../../assets/logo-dark.svg">
  <source media="(prefers-color-scheme: light)" srcset="../../assets/logo-light.svg">
  <img alt="solidcraft-parasolid logo" src="../../assets/logo-dark.svg" width="460">
</picture>

<p>
  <strong>Clean-Room-Importer-Plugin für Parasolid XT (<code>.x_b</code>, <code>.x_t</code>, <code>.x_p</code>) in SolidCraft — reines Rust &amp; WebAssembly.</strong>
</p>

<p>
  <a href="../../README.md">English</a> ·
  <a href="README.ru.md">Русский</a> ·
  <a href="README.zh-CN.md">简体中文</a> ·
  <a href="README.de.md">Deutsch</a> ·
  <a href="README.ja.md">日本語</a> ·
  <a href="README.es.md">Español</a>
</p>

</div>

---

## Überblick

Der Kern von [SolidCraft](https://github.com/tangency-labs/solidcraft) unterstützt nur offene Formate (`STEP`, `IGES`, `3MF`). Proprietäre Austauschformate wie **Parasolid XT** (`.x_b`, `.x_t`, `.x_p`) werden über **WebAssembly-Importer-Plugins** eingebunden.

`solidcraft-parasolid` ist eine reine Rust-Portierung von [xtopen](https://github.com/41Opty41/xtopen) als **`craft:plugin@0.2.0` WebAssembly Component** (`wasm32-unknown-unknown`, ohne WASI und ohne Netzwerkzugriff):

- Öffnet `.x_b`, `.x_t`, `.x_p`, `.xmt_txt` und `.xmt_bin` direkt über **File › Open** in SolidCraft `v0.50.0+` ([tangency-labs/solidcraft#22054](https://github.com/tangency-labs/solidcraft/pull/22054)).
- Enthält ein eigenes Parasolid-XT-Basisschema (Form-1/Form-2 binär und Text) sowie einen eigenen STEP-AP214/AP242-Writer ohne externe CAD-Kernels.
- **8,5–13,3× schneller** als Python `xtopen` nativ; End-to-End-Import im SolidCraft-WASM-Host in **65–129 ms** für typische Einzelteile.

## Installation in SolidCraft (`v0.50.0+`)

1. **`solidcraft-parasolid-0.1.0.zip`** von den [Releases](https://github.com/41Opty41/solidcraft-parasolid/releases/latest) herunterladen.
2. In SolidCraft unter **Preferences › Plugins › Install plugin…** das `.zip`-Archiv auswählen.
3. `.x_b` / `.x_t` / `.x_p` direkt über **File › Open** öffnen.

## Lizenz & Unterstützung

Doppellizenz **MIT OR Apache-2.0** ([`LICENSE-MIT`](../../LICENSE-MIT), [`LICENSE-APACHE`](../../LICENSE-APACHE)). Hinweise zu Spenden in [`DONATE.md`](../../DONATE.md).
