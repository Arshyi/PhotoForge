# Drives EXR, SVG and PSD in the packaged window: each file is opened through the real Open button,
# what the app said is recorded (the report dialog, the IPC commands and any error), and then it is
# exported through the real Export button to the formats that apply. The native file dialogs, which
# cannot be automated, are answered at the IPC call by cdp.mjs.
#
# Needs: a release executable (see README.md), node, and the sample files:
#   python tools/fetch_exr_samples.py     -> <work>\exr-samples
#   python tools/fetch_svg_samples.py     -> <work>\svg-samples
#   python tools/fetch_psd_samples.py     -> <work>\psd-samples\psd-tools
# The outputs are checked afterwards with tools\validate\check_formats.py.
param([string]$Exe = '')
. "$PSScriptRoot\env.ps1"
if (-not $Exe) { $Exe = $PfExe }
$outDir = Join-Path $PfValidate 'out\formats'
New-Item -ItemType Directory -Force $outDir | Out-Null
Set-Location $PfValidate
$results = New-Object System.Collections.Generic.List[string]
function Say($t) { $results.Add($t); Write-Output $t }

$dialogJs = @'
(() => {
  const fixed = [...document.querySelectorAll('body *')].filter((e) => {
    const s = getComputedStyle(e); const r = e.getBoundingClientRect();
    return s.position === 'fixed' && r.width > 200 && r.height > 100 && e.innerText;
  });
  const top = fixed.length ? fixed[fixed.length - 1].innerText.replace(/\s+/g, ' ').trim().slice(0, 700) : '';
  const status = [...document.querySelectorAll('body *')].filter((e) => /· \d+% · \d+ operations/.test(e.textContent) && e.children.length < 6).map((e) => e.textContent.trim()).slice(-1)[0] || '';
  const toast = [...document.querySelectorAll('[role=status],[role=alert]')].map((e) => e.innerText.trim()).filter(Boolean).slice(0, 3);
  return JSON.stringify({ dialog: top, status, toast });
})()
'@

function Node-Eval($js) { (node $PfCdp eval $js) -join "`n" }
function Ipc-Summary() {
    $entries = ((node $PfCdp log) -join "`n") | ConvertFrom-Json
    $errs = @($entries | Where-Object { $_.error })
    $cmds = @($entries | Where-Object { $_.command -notmatch 'dialog' } | ForEach-Object { $_.command }) -join ','
    [pscustomobject]@{ Commands = $cmds; Errors = ($errs | ForEach-Object { "$($_.command): $($_.error)" }) }
}
function Close-Dialog() { node $PfCdp clickText Close 2>$null | Out-Null; Start-Sleep -Milliseconds 700 }

function Open-File($path, $tag, $wait = 8) {
    node $PfCdp eval "window.__pf.nextOpen = '$path'; window.__pf.log.length = 0; 1" | Out-Null
    node $PfCdp clickText Open | Out-Null
    Start-Sleep -Seconds $wait
    node $PfCdp shot "shots/fmt-$tag-open.png" | Out-Null
    $state = (Node-Eval $dialogJs | ConvertFrom-Json) | ConvertFrom-Json
    $ipc = Ipc-Summary
    Say "OPEN  $tag"
    if ($state.dialog) { Say "  dialog: $($state.dialog)" }
    if ($state.toast) { Say "  toast : $($state.toast -join ' | ')" }
    Say "  ipc   : $($ipc.Commands)"
    $ipc.Errors | ForEach-Object { Say "  ERROR : $_" }
    Close-Dialog
}

function Export-To($outPath, $tag) {
    if (Test-Path $outPath) { Remove-Item $outPath -Force }
    $fwd = $outPath -replace '\\', '/'
    node $PfCdp eval "window.__pf.nextSave = '$fwd'; window.__pf.log.length = 0; 1" | Out-Null
    node $PfCdp clickText Export | Out-Null
    Start-Sleep -Seconds 6
    $state = (Node-Eval $dialogJs | ConvertFrom-Json) | ConvertFrom-Json
    $ipc = Ipc-Summary
    $size = if (Test-Path $outPath) { (Get-Item $outPath).Length } else { -1 }
    Say "EXPORT $tag -> $([IO.Path]::GetFileName($outPath)): size=$size ; ipc: $($ipc.Commands)"
    if ($state.dialog) { Say "  dialog: $($state.dialog)" }
    $ipc.Errors | ForEach-Object { Say "  ERROR : $_" }
    Close-Dialog
}

# launch and attach
$o = & "$PSScriptRoot\launch.ps1" -Exe $Exe
$procId = [int](($o | Select-String 'PID (\d+)').Matches[0].Groups[1].Value)
Start-Sleep -Seconds 8
node $PfCdp install | Out-Null
# A document with unsaved work asks through window.confirm (a native dialog that would stop the
# page); answer yes so a sequence of opens is not held up. Nothing else about the flow changes.
node $PfCdp eval "window.confirm = () => true; 1" | Out-Null

$work = ($PfWork -replace '\\', '/')
$exr = "$work/exr-samples"
$svg = "$work/svg-samples"
$psd = "$work/psd-samples/psd-tools/tests/psd_files"

# EXR
foreach ($name in 'TestImages__SquaresSwirls', 'TestImages__WideColorGamut', 'ScanLines__Desk') {
    Open-File "$exr/$name.exr" "exr-$name" 10
    Export-To (Join-Path $outDir "$name.exr") "exr-$name"
    Export-To (Join-Path $outDir "$name.png") "exr-$name"
}
Open-File "$exr/LuminanceChroma__Flowers.exr" 'exr-refused-luminance-chroma' 8
Open-File "$exr/TestImages__BrightRingsNanInf.exr" 'exr-naninf' 8

# SVG
foreach ($name in 'bootstrap-airplane-fill', 'bootstrap-apple-music', 'bootstrap-8-circle-fill') {
    Open-File "$svg/$name.svg" "svg-$name" 6
    Export-To (Join-Path $outDir "$name.svg") "svg-$name"
    Export-To (Join-Path $outDir "$name.psd") "svg-$name"
}

# PSD
foreach ($name in '2layers', 'adjustment-fillers', 'layer_effects', '32bit') {
    Open-File "$psd/$name.psd" "psd-$name" 8
    Export-To (Join-Path $outDir "psd-$name-out.psd") "psd-$name"
}

# close
$p = Get-Process -Id $procId -ErrorAction SilentlyContinue
if ($p) {
    $null = $p.CloseMainWindow()
    for ($i = 0; $i -lt 20; $i++) { Start-Sleep -Milliseconds 500; $p.Refresh(); if ($p.HasExited) { break } }
    Say "CLOSE exited=$($p.HasExited) after ~$([math]::Round($i*0.5,1))s"
    if (-not $p.HasExited) { Stop-Process -Id $procId -Force }
}
$results | Set-Content (Join-Path $PfValidate 'formats-report.txt')
