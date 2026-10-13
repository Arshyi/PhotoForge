# Repeats: launch the release executable, open a portrait RAW region with the render commands made
# to fail (the state the 15.9 defect produced), then ask the window to close with WM_CLOSE and
# report whether it exited. The backend's own record of each request is
# <validate data>\localappdata\PhotoForge\diagnostics\close.log (see docs\phase-15-9-3-results.md).
#
# -Runs         how many launches
# -GapSeconds   idle time between the document settling and the close request
# -Inject:$false  do not make the render commands fail (a healthy document)
# -Observe      also register a close-requested listener in the page, and print what it saw
param(
    [int]$Runs = 4,
    [bool]$Inject = $true,
    [int]$SettleSeconds = 12,
    [int]$GapSeconds = 0,
    [switch]$Observe
)
. "$PSScriptRoot\env.ps1"
$inputs = ($PfValidate -replace '\\', '/') + '/inputs'
if (-not (Test-Path (Join-Path $PfValidate 'inputs\portrait.dng'))) { throw "No RAW inputs in $PfValidate\inputs. Run: node tools\validate\make_inputs.mjs" }
$injector = @"
(() => { const f = window.fetch; window.fetch = async (input, init) => { const url = typeof input === 'string' ? input : input.url; if (url.includes('ipc.localhost')) { const c = decodeURIComponent(new URL(url).pathname.slice(1)); if (['render_layer_thumbnail','analyze_image','generate_histogram','render_layer_composite','render_preview'].includes(c)) { window.__pf.log.push({command: c + ' (injected error)'}); return new Response(JSON.stringify({code:'invalid_layer_document',message:'injected'}), {status: 200, headers: {'Content-Type':'application/json','Tauri-Response':'error'}}); } } return f(input, init); }; return 'injected'; })()
"@
for ($run = 1; $run -le $Runs; $run++) {
    $out = & "$PSScriptRoot\launch.ps1"
    $procId = [int](($out | Select-String 'PID (\d+)').Matches[0].Groups[1].Value)
    Start-Sleep -Seconds 8
    Push-Location $PfValidate
    node $PfCdp install | Out-Null
    if ($Inject) { node $PfCdp eval $injector | Out-Null }
    if ($Observe) { node $PfCdp evalfile (Join-Path $PfTools 'listen_close.js') | Out-Null }
    node $PfCdp eval "window.__pf.nextOpen = '$inputs/portrait.dng'; 1" | Out-Null
    node $PfCdp clickText Open | Out-Null; Start-Sleep -Seconds 4
    node $PfCdp clickText 'Choose Region' | Out-Null; Start-Sleep -Seconds 7
    node $PfCdp clickText 'Open region' | Out-Null; Start-Sleep -Seconds $SettleSeconds
    $p = Get-Process -Id $procId -ErrorAction SilentlyContinue
    Start-Sleep -Seconds $GapSeconds
    $sent = $p.CloseMainWindow()
    for ($i = 0; $i -lt 20; $i++) { Start-Sleep -Milliseconds 500; $p.Refresh(); if ($p.HasExited) { break } }
    $exited = $p.HasExited
    $detail = ''
    if (-not $exited) {
        $state = node $PfCdp evalfile (Join-Path $PfTools 'close_state.js')
        node $PfCdp shot "shots/repro-$run-notclosed.png" | Out-Null
        $detail = "page state: $state"
        Stop-Process -Id $procId -Force
    }
    Pop-Location
    "run ${run}: inject=$Inject sent=$sent exited=$exited after ~$([math]::Round($i*0.5,1))s $detail"
    Start-Sleep -Seconds 2
}
$log = Join-Path $PfValidate 'localappdata\PhotoForge\diagnostics\close.log'
if (Test-Path $log) { "--- close.log"; Get-Content $log }
