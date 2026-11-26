<div align="center">

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="../../assets/logo-dark.svg">
  <source media="(prefers-color-scheme: light)" srcset="../../assets/logo-light.svg">
  <img alt="solidcraft-parasolid logo" src="../../assets/logo-dark.svg" width="460">
</picture>

<p>
  <strong>Плагин-импортёр B-rep моделей Parasolid XT (<code>.x_b</code>, <code>.x_t</code>, <code>.x_p</code>) для SolidCraft — на чистом Rust и WebAssembly без проприетарного кода.</strong>
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

## Зачем нужен `solidcraft-parasolid`

В ядро [SolidCraft](https://github.com/tangency-labs/solidcraft) по правилам проекта принимаются только открытые форматы (`STEP`, `IGES`, `3MF`). Поддержка коммерческих CAD-форматов вроде **Parasolid XT** (`.x_b`, `.x_t`, `.x_p`) вынесена в изолированные **WebAssembly-плагины**.

`solidcraft-parasolid` — это чистый Rust-порт [xtopen](https://github.com/41Opty41/xtopen), скомпилированный в компонент WebAssembly **`craft:plugin@0.2.0`** (`wasm32-unknown-unknown`, без WASI и без доступа к сети):

- **Открывает `.x_b`, `.x_t`, `.x_p`, `.xmt_txt`, `.xmt_bin` напрямую** через **File › Open** в SolidCraft `v0.50.0+` (поддержка подчёркиваний в расширениях принята в [tangency-labs/solidcraft#22054](https://github.com/tangency-labs/solidcraft/pull/22054)).
- **Clean-room и автономность:** встроенная базовая схема Parasolid XT (бинарный Form-1/Form-2 и текстовый форматы), без библиотек вендоров и без OpenCASCADE.
- **Точная аналитическая геометрия и NURBS:** плоскости, цилиндры, конусы, сферы, торы, поверхности выдавливания/вращения/смещения, прямые, окружности, эллипсы, рациональные и обычные NURBS, 2D p-кривые.
- **Скорость:** в **8.5–13.3 раза быстрее** Python-версии нативно и **65–129 мс** «под ключ» внутри песочницы SolidCraft для типовых деталей.

## Установка в SolidCraft (`v0.50.0+`)

1. Скачайте **`solidcraft-parasolid-0.1.0.zip`** со страницы [Releases](https://github.com/41Opty41/solidcraft-parasolid/releases/latest).
2. В SolidCraft откройте **Preferences › Plugins › Install plugin…** и выберите скачанный `.zip`.
3. Открывайте файлы `.x_b`, `.x_t`, `.x_p`, `.xmt_txt`, `.xmt_bin` напрямую через **File › Open** (при необходимости выберите схему STEP `AP214`/`AP242` и единицы `mm`/`m`).

## Проверка и скорость

- `solidcraft-cli plugin check` (`0.50.0`): `0` ошибок, `0` предупреждений, ноль WASI-импортов.
- `solidcraft-cli plugin test --require-cases` (`0.50.0`): **10/10** тестов пройдено (включая проверку объёмов ядром SolidCraft).
- Перекрёстная проверка в OpenCASCADE: **8/8** моделей валидны, расхождение объёма и площади `0.0e+00`.
- Подробная таблица бенчмарков приведена в основном [README](../../README.md#benchmark-python-xtopen-vs-native-rust-vs-solidcraft-wasm).

## Поддержка и лицензия

Двойная лицензия **MIT OR Apache-2.0** ([`LICENSE-MIT`](../../LICENSE-MIT), [`LICENSE-APACHE`](../../LICENSE-APACHE)).  
Поддержать разработку (Bitcoin / ЮMoney): [`DONATE.md`](../../DONATE.md).
