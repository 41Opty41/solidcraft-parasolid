<div align="center">

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="../../assets/logo-dark.svg">
  <source media="(prefers-color-scheme: light)" srcset="../../assets/logo-light.svg">
  <img alt="solidcraft-parasolid logo" src="../../assets/logo-dark.svg" width="460">
</picture>

<p>
  <strong>Plugin importador limpio de B-rep Parasolid XT (<code>.x_b</code>, <code>.x_t</code>, <code>.x_p</code>) para SolidCraft — escrito en Rust puro y WebAssembly.</strong>
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

## Acerca de `solidcraft-parasolid`

El núcleo de [SolidCraft](https://github.com/tangency-labs/solidcraft) solo incluye formatos abiertos (`STEP`, `IGES`, `3MF`). Los formatos comerciales como **Parasolid XT** (`.x_b`, `.x_t`, `.x_p`) se integran mediante **plugins importadores en WebAssembly**.

`solidcraft-parasolid` es un port en Rust puro de [xtopen](https://github.com/41Opty41/xtopen) compilado como componente WebAssembly **`craft:plugin@0.2.0`** (`wasm32-unknown-unknown`, sin WASI ni acceso a red):

- **Abre `.x_b`, `.x_t`, `.x_p`, `.xmt_txt`, `.xmt_bin` directamente** con **File › Open** en SolidCraft `v0.50.0+` ([tangency-labs/solidcraft#22054](https://github.com/tangency-labs/solidcraft/pull/22054)).
- **Clean-room y autónomo:** esquema base Parasolid XT integrado (binario Form-1/Form-2 y texto) + exportador STEP AP214/AP242 sin dependencias de núcleos externos.
- **8,5–13,3× más rápido** que Python `xtopen` de forma nativa y **65–129 ms** de principio a fin dentro del sandbox WASM de SolidCraft.

## Instalación en SolidCraft (`v0.50.0+`)

1. Descarga **`solidcraft-parasolid-0.1.0.zip`** desde [Releases](https://github.com/41Opty41/solidcraft-parasolid/releases/latest).
2. En SolidCraft ve a **Preferences › Plugins › Install plugin…** y selecciona el archivo `.zip`.
3. Abre cualquier archivo `.x_b`, `.x_t` o `.x_p` directamente desde **File › Open**.

## Licencia y apoyo

Licencia dual **MIT OR Apache-2.0** ([`LICENSE-MIT`](../../LICENSE-MIT) / [`LICENSE-APACHE`](../../LICENSE-APACHE)). Donaciones voluntarias en [`DONATE.md`](../../DONATE.md).
