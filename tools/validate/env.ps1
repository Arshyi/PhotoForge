# Dot-sourced by the other scripts in this directory: where things are, from the repository layout,
# so a clone on any machine works with no editing.
#
#   $PfTools     this directory
#   $PfRepo      the repository root
#   $PfWork      scratch (corpora, screenshots, outputs); PF_WORK moves it; <repo>\.work by default
#   $PfValidate  $PfWork\validate: the validation run's own data folders, screenshots and outputs
#   $PfExe       the release executable: PF_TARGET_DIR (or CARGO_TARGET_DIR) \release\photoforge.exe,
#                else src-tauri\target\release\photoforge.exe
#   $PfBundles   the installers' folder, next to it
#   $PfCdp       the DevTools-protocol client (cdp.mjs)
$script:PfTools = $PSScriptRoot
$script:PfRepo = (Resolve-Path (Join-Path $PSScriptRoot '..\..')).Path
$script:PfWork = if ($env:PF_WORK) { $env:PF_WORK } else { Join-Path $script:PfRepo '.work' }
$script:PfValidate = Join-Path $script:PfWork 'validate'
$pfTarget = if ($env:PF_TARGET_DIR) { $env:PF_TARGET_DIR } elseif ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { Join-Path $script:PfRepo 'src-tauri\target' }
$script:PfReleaseDir = Join-Path $pfTarget 'release'
$script:PfExe = Join-Path $script:PfReleaseDir 'photoforge.exe'
$script:PfBundles = Join-Path $script:PfReleaseDir 'bundle'
$script:PfCdp = Join-Path $PSScriptRoot 'cdp.mjs'
New-Item -ItemType Directory -Force $script:PfValidate,
    (Join-Path $script:PfValidate 'shots'), (Join-Path $script:PfValidate 'out') | Out-Null
