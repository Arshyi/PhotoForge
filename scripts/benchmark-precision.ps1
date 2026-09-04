param([string]$Executable)
$ErrorActionPreference = 'Stop'
if (-not $Executable) {
    $Executable = Join-Path $PSScriptRoot '..\src-tauri\target\release\examples\precision_benchmark.exe'
}
$Executable = (Resolve-Path -LiteralPath $Executable).Path
# Build with cargo build --release --example precision_benchmark first.
# Each invocation is a new process, so Windows reports a fresh high-water mark.
foreach ($dimensions in @(@(4240,2832), @(6000,4000), @(8256,5504), @(9504,6336))) {
    foreach ($scenario in @('raw','single','adjustment','mask','multi','transform','preview','export','legacy')) {
        & $Executable $dimensions[0] $dimensions[1] $scenario
        if ($LASTEXITCODE -ne 0) { throw "Benchmark failed: $dimensions $scenario" }
    }
}
