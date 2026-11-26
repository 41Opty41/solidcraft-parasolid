<div align="center">

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="../../assets/logo-dark.svg">
  <source media="(prefers-color-scheme: light)" srcset="../../assets/logo-light.svg">
  <img alt="solidcraft-parasolid logo" src="../../assets/logo-dark.svg" width="460">
</picture>

<p>
  <strong>SolidCraft 向けクリーンルーム Parasolid XT (<code>.x_b</code>, <code>.x_t</code>, <code>.x_p</code>) B-rep インポータープラグイン — 純粋 Rust &amp; WebAssembly 実装。</strong>
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

## 概要

[SolidCraft](https://github.com/tangency-labs/solidcraft) 本体はオープン規格（`STEP`、`IGES`、`3MF`）のみを内蔵し、**Parasolid XT**（`.x_b`、`.x_t`、`.x_p`）などの商用形式はサンドボックス化された **WebAssembly インポータープラグイン** として追加されます。

`solidcraft-parasolid` は [xtopen](https://github.com/41Opty41/xtopen) を純粋 Rust に移植した **`craft:plugin@0.2.0` WebAssembly Component**（`wasm32-unknown-unknown`、WASI 不使用・ネットワーク権限不要）です。

- SolidCraft `v0.50.0+` の **File › Open** から `.x_b` / `.x_t` / `.x_p` / `.xmt_txt` / `.xmt_bin` を直接開けます（[tangency-labs/solidcraft#22054](https://github.com/tangency-labs/solidcraft/pull/22054) によりアンダースコア付き拡張子に対応）。
- 組み込み XT スキーマ（Form-1/Form-2 バイナリおよびテキスト）と STEP AP214/AP242 ライターを内蔵し、外部 CAD カーネル不要。
- ネイティブ実行では Python 版 `xtopen` より **8.5〜13.3 倍高速**、SolidCraft WASM ホスト上でも単一部品を **65〜129 ms** でインポートできます。

## インストール方法（SolidCraft `v0.50.0+`）

1. [Releases](https://github.com/41Opty41/solidcraft-parasolid/releases/latest) から **`solidcraft-parasolid-0.1.0.zip`** をダウンロードします。
2. SolidCraft の **Preferences › Plugins › Install plugin…** から `.zip` を選択してインストールします。
3. **File › Open** から `.x_b` / `.x_t` / `.x_p` を直接開きます。

## ライセンスと支援

**MIT OR Apache-2.0** デュアルライセンス（[`LICENSE-MIT`](../../LICENSE-MIT) / [`LICENSE-APACHE`](../../LICENSE-APACHE)）。寄付については [`DONATE.md`](../../DONATE.md) をご覧ください。
