<div align="center">

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="../../assets/logo-dark.svg">
  <source media="(prefers-color-scheme: light)" srcset="../../assets/logo-light.svg">
  <img alt="solidcraft-parasolid logo" src="../../assets/logo-dark.svg" width="460">
</picture>

<p>
  <strong>面向 SolidCraft 的净室 Parasolid XT（<code>.x_b</code>、<code>.x_t</code>、<code>.x_p</code>）B-rep 导入插件 —— 纯 Rust 编写并编译为 WebAssembly。</strong>
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

## 简介

[SolidCraft](https://github.com/tangency-labs/solidcraft) 核心仅内置开放格式（`STEP`、`IGES`、`3MF`），商业格式如 **Parasolid XT**（`.x_b`、`.x_t`、`.x_p`）通过沙箱化的 **WebAssembly 导入插件** 扩展支持。

`solidcraft-parasolid` 是 [xtopen](https://github.com/41Opty41/xtopen) 的纯 Rust 移植版本，编译为 **`craft:plugin@0.2.0` WebAssembly Component**（`wasm32-unknown-unknown`，无 WASI 依赖，无网络访问）：

- **在 SolidCraft `v0.50.0+` 中通过 File › Open 直接打开 `.x_b`、`.x_t`、`.x_p`、`.xmt_txt`、`.xmt_bin`**（下划线扩展名支持已合并至上游 [tangency-labs/solidcraft#22054](https://github.com/tangency-labs/solidcraft/pull/22054)）。
- **净室自包含实现：** 内置 Parasolid XT 基础 Schema（支持 Form-1/Form-2 二进制与文本流），零商业内核依赖。
- **完整几何支持：** 平面、圆柱、圆锥、球面、圆环面、拉伸/旋转/偏置曲面、直线、圆、椭圆、有理/非有理 NURBS 及 2D 参数裁剪曲线。
- **高性能：** 原生速度比 Python `xtopen` 快 **8.5–13.3 倍**，常规零件在 SolidCraft WASM 沙箱端到端导入耗时仅 **65–129 ms**。

## 在 SolidCraft (`v0.50.0+`) 中安装

1. 从 [Releases](https://github.com/41Opty41/solidcraft-parasolid/releases/latest) 下载 **`solidcraft-parasolid-0.1.0.zip`**。
2. 在 SolidCraft 中打开 **Preferences › Plugins › Install plugin…** 并选择该 `.zip` 压缩包。
3. 直接通过 **File › Open** 打开 `.x_b`、`.x_t`、`.x_p` 等文件。

## 验证与许可

- SolidCraft `0.50.0` `plugin check` 与 `plugin test --require-cases`：**10/10** 用例全部通过。
- 采用 **MIT OR Apache-2.0** 双重许可（[`LICENSE-MIT`](../../LICENSE-MIT) / [`LICENSE-APACHE`](../../LICENSE-APACHE)），赞助支持请参阅 [`DONATE.md`](../../DONATE.md)。
