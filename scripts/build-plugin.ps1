$ErrorActionPreference = "Stop"
$env:PATH = "$env:USERPROFILE\.cargo\bin;" + $env:PATH

$root = Split-Path -Parent $PSScriptRoot
Push-Location $root
try {
    cargo build --release -p xtopen-cli
    cargo build --release --target wasm32-unknown-unknown -p solidcraft-parasolid-wasm

    $wasmTools = Get-ChildItem (Join-Path $root ".tools\wasm-tools") -Recurse -Filter "wasm-tools.exe" | Select-Object -First 1
    if (-not $wasmTools) {
        throw "wasm-tools.exe not found under $root\.tools\wasm-tools"
    }
    $coreWasm = Join-Path $root "target\wasm32-unknown-unknown\release\solidcraft_parasolid_wasm.wasm"
    $compWasm = Join-Path $root "solidcraft-parasolid\solidcraft-parasolid.wasm"
    & $wasmTools.FullName component new $coreWasm -o $compWasm
    if ($LASTEXITCODE -ne 0) {
        throw "wasm-tools component new failed"
    }

    $zipPath = Join-Path $root "solidcraft-parasolid-0.1.0.zip"
    if (Test-Path $zipPath) { Remove-Item $zipPath -Force }
    Compress-Archive -Path @(
        (Join-Path $root "solidcraft-parasolid\plugin.json"),
        $compWasm
    ) -DestinationPath $zipPath

    Write-Output "Built component: $compWasm ($( (Get-Item $compWasm).Length ) bytes)"
    Write-Output "Built zip archive: $zipPath ($( (Get-Item $zipPath).Length ) bytes)"
}
finally {
    Pop-Location
}
