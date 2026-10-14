# Shows the packaged window at a matrix of widths, heights and device pixel ratios, with a
# document open, and says what is wrong at each: the page scrolling sideways, a header button past
# the window's edge, the editing panel's controls outside it or clipping their text, and controls
# drawn with the browser's own look. A screenshot of each is left in <work>\validate\shots.
#
# The window is not resized and the display's scale is not changed (that is a system setting): the
# page is shown at a given size and device pixel ratio with DevTools' device metrics override, which
# is what CSS layout depends on. It does not exercise how Windows scales the window itself.
#
#   -Sizes   WxH pairs, default 760x560 (the minimum window), 900x700, 1100x760, 1280x720,
#            1366x768, 1400x800, 1536x864, 1920x1080
#   -Ratios  default 1, 1.25, 1.5, 2
#
# Exit 1 if any cell has a problem.
param(
    [string[]]$Sizes = @('760x560', '900x700', '1100x760', '1280x720', '1366x768', '1400x800', '1536x864', '1920x1080'),
    [double[]]$Ratios = @(1, 1.25, 1.5, 2)
)
$ErrorActionPreference = 'Stop'
. "$PSScriptRoot\env.ps1"
$inputs = Join-Path $PfValidate 'inputs'
$picture = Join-Path $inputs 'clip-source.png'
if (-not (Test-Path $picture)) { node (Join-Path $PfTools 'make_pictures.mjs') | Out-Null }

$procId = $null
$failed = 0
try {
    $out = & "$PSScriptRoot\launch.ps1"
    $procId = [int](($out | Select-String 'PID (\d+)').Matches[0].Groups[1].Value)
    Start-Sleep -Seconds 8
    Push-Location $PfValidate
    try {
        node $PfCdp install | Out-Null
        node $PfCdp eval "window.__pf.nextOpen = '$(($picture) -replace '\\', '/')'; 1" | Out-Null
        node $PfCdp clickText Open | Out-Null
        Start-Sleep -Seconds 3
        $rows = foreach ($size in $Sizes) {
            $w, $h = $size -split 'x'
            foreach ($ratio in $Ratios) {
                $name = "shots/layout-${w}x${h}-dpr$ratio.png"
                $result = node $PfCdp layout $w $h $ratio $name | ConvertFrom-Json
                if ($result.problems.Count -gt 0) { $failed += 1 }
                [pscustomobject]@{
                    Size = $size; Ratio = $ratio; Controls = $result.controls
                    Problems = $result.problems.Count
                    Detail = ($result.problems | Select-Object -First 2) -join ' | '
                }
            }
        }
        $rows | Format-Table -AutoSize -Wrap | Out-String -Width 200 | Write-Host
    }
    finally { Pop-Location }
}
finally {
    if ($procId) {
        $p = Get-Process -Id $procId -ErrorAction SilentlyContinue
        if ($p) {
            $null = $p.CloseMainWindow()
            for ($i = 0; $i -lt 20 -and -not $p.HasExited; $i++) { Start-Sleep -Milliseconds 500; $p.Refresh() }
            if (-not $p.HasExited) { Stop-Process -Id $procId -Force }
        }
    }
}
Write-Host "$failed of $($Sizes.Count * $Ratios.Count) cells have a problem"
if ($failed -gt 0) { exit 1 }
