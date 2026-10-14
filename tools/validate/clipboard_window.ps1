# Drives PhotoForge's clipboard in the packaged window, with real key events, and reads the
# clipboard from outside (clipboard_probe.py) to say what was put on it, and writes it from outside
# to say what is read from it.
#
#   1. Ctrl+C       copy the layer: the PNG form and CF_DIBV5 are the picture trimmed to where it has
#                   anything, pixel for pixel, with the cloud-clipboard opt-out
#   2. Ctrl+Shift+C copy the merged picture: the whole 96 x 64 canvas
#   3. Ctrl+V       a picture another program put there (PNG + a 24-bit CF_DIB) becomes a new layer;
#                   the layer is then copied again and must be that picture exactly
#   4. Ctrl+V       a clipboard with only a 24-bit CF_DIB
#   5. Ctrl+V       a clipboard with only text: refused in a sentence, nothing placed
#   6. the palette  "New document from clipboard" opens the picture as a new document
#
# THE CLIPBOARD IS REPLACED. It is saved first with tools\clipboard_backup.ps1 (every format that
# lives in global memory), and put back at the end, whatever happens; if it holds something that
# cannot be put back the script refuses to start (-AllowLoss overrides).
#
# Needs: the release executable (see README.md), .work\venv with tools\requirements.txt (Pillow),
# node. Run from anywhere.
param([switch]$AllowLoss)
$ErrorActionPreference = 'Stop'
. "$PSScriptRoot\env.ps1"

$python = Join-Path $PfRepo '.work\venv\Scripts\python.exe'
if (-not (Test-Path $python)) { $python = 'python' }
$probe = Join-Path $PfTools 'clipboard_probe.py'
$backupScript = Join-Path $PfRepo 'tools\clipboard_backup.ps1'
$backupDir = Join-Path $PfValidate 'clipboard-backup'
$inputs = Join-Path $PfValidate 'inputs'
$source = Join-Path $inputs 'clip-source.png'
$other = Join-Path $inputs 'clip-other.png'
if (-not ((Test-Path $source) -and (Test-Path $other))) { node (Join-Path $PfTools 'make_pictures.mjs') | Out-Null }

$results = New-Object System.Collections.ArrayList
function Check($name, $ok, $detail) {
    [void]$results.Add([pscustomobject]@{ Name = $name; Ok = [bool]$ok; Detail = $detail })
    $mark = if ($ok) { 'ok  ' } else { 'FAIL' }
    Write-Host ("{0} {1}{2}" -f $mark, $name, $(if ($detail) { " - $detail" } else { '' }))
}
function Probe { & $python -I $probe @args 2>&1 }
function Cdp { Push-Location $PfValidate; try { node $PfCdp @args } finally { Pop-Location } }
function PageText { (Cdp text) -join "`n" }
function WaitFor($pattern, $seconds = 10) {
    $until = (Get-Date).AddSeconds($seconds)
    do {
        $text = PageText
        if ($text -match $pattern) { return $true }
        Start-Sleep -Milliseconds 250
    } while ((Get-Date) -lt $until)
    return $false
}
function IpcErrors {
    $log = (Cdp log) -join "`n" | ConvertFrom-Json
    @($log | Where-Object { $_.status -and ($_.status -ne 200 -or $_.tauri -ne 'ok') })
}

# --- the person's clipboard, first
$saved = & powershell.exe -NoProfile -ExecutionPolicy Bypass -File $backupScript -Action save -Path $backupDir @(if ($AllowLoss) { '-AllowLoss' })
$saved | Select-Object -Last 2 | ForEach-Object { Write-Host $_ }
if ($LASTEXITCODE -ne 0) { throw 'The clipboard cannot be saved whole, so it will not be touched. See above.' }

$procId = $null
try {
    $out = & "$PSScriptRoot\launch.ps1"
    $procId = [int](($out | Select-String 'PID (\d+)').Matches[0].Groups[1].Value)
    Start-Sleep -Seconds 8
    Cdp install | Out-Null
    Cdp eval "window.confirm = () => true; window.__pf.nextOpen = '$(($source) -replace '\\', '/')'; 1" | Out-Null
    Cdp clickText Open | Out-Null
    Check 'the picture opens' (WaitFor 'Hide Background|Background' 10) ''

    # 1. copy the layer (over some text, which a copy must replace, not sit beside)
    $null = Probe set-text 'text that a copy must replace'
    $seq = (Probe seq)
    Cdp chord ctrl+c | Out-Null
    $changed = (Probe wait-change $seq 10) -join ''
    $report = (Probe check-copy $source layer) -join '' | ConvertFrom-Json
    Check 'Ctrl+C: the clipboard changed' ($changed -match '"changed": true') ''
    Check 'Ctrl+C: PNG and CF_DIBV5 are the layer, trimmed, pixel for pixel' ($report.problems.Count -eq 0) ("PNG $($report.png -join ' x ') DIBV5 $($report.dibv5 -join ' x ') cloud opt-out $($report.cloud_opt_out); " + ($report.problems -join '; '))
    $names = @((Probe formats) -join '' | ConvertFrom-Json | ForEach-Object { $_.name })
    Check 'Ctrl+C: the text that was there is gone, not kept beside the picture' (-not ($names -contains 'CF_UNICODETEXT' -or $names -contains 'CF_TEXT')) ($names -join ', ')
    Check 'Ctrl+C: the notification says what was copied' (WaitFor 'Copied the layer \(80 .{1,4} 48\) to the clipboard') ''

    # 2. copy the merged picture
    $seq = (Probe seq)
    Cdp chord ctrl+shift+c | Out-Null
    $null = Probe wait-change $seq 10
    $report = (Probe check-copy $source merged) -join '' | ConvertFrom-Json
    Check 'Ctrl+Shift+C: the whole canvas, pixel for pixel' ($report.problems.Count -eq 0) ("PNG $($report.png -join ' x ') DIBV5 $($report.dibv5 -join ' x '); " + ($report.problems -join '; '))
    Check 'Ctrl+Shift+C: the notification' (WaitFor 'Copied the picture \(96 .{1,4} 64\) to the clipboard') ''

    # 3. paste a picture another program put there (PNG with alpha + 24-bit CF_DIB)
    $null = Probe set-image $other png,dib
    Cdp chord ctrl+v | Out-Null
    Check 'Ctrl+V: a picture from another program becomes a layer' (WaitFor 'Pasted a 40 .{1,4} 30 picture from the clipboard as a new layer') ''
    # The pasted layer is the active one: copy it back out and it must be that picture, exactly.
    $seq = (Probe seq)
    Cdp chord ctrl+c | Out-Null
    $null = Probe wait-change $seq 10
    $report = (Probe check-copy $other layer) -join '' | ConvertFrom-Json
    Check 'the pasted layer is the picture that was on the clipboard, pixel for pixel' ($report.problems.Count -eq 0) ($report.problems -join '; ')

    # 4. a clipboard with only a 24-bit CF_DIB
    $null = Probe set-image $other dib
    Cdp chord ctrl+v | Out-Null
    Check 'Ctrl+V: a lone CF_DIB is read' (WaitFor 'Pasted a 40 .{1,4} 30 picture from the clipboard as a new layer') ''

    # 5. text only
    $null = Probe set-text 'not a picture'
    Cdp chord ctrl+v | Out-Null
    Check 'Ctrl+V: text is refused in a sentence' (WaitFor 'The clipboard does not hold a picture') ''

    # 6. the palette: a new document from the clipboard
    $null = Probe set-image $other png,dib
    Cdp chord ctrl+shift+p | Out-Null
    Start-Sleep -Milliseconds 600
    Cdp insert 'New document from clipboard' | Out-Null
    Start-Sleep -Milliseconds 600
    Cdp chord enter | Out-Null
    Check 'the palette: a new document from the clipboard' (WaitFor 'Opened the 40 .{1,4} 30 picture from the clipboard as a new document') ''

    # The only command allowed to fail is the paste of text (step 5), and exactly once.
    $errors = IpcErrors
    $unexpected = @($errors | Where-Object { $_.command -ne 'paste_clipboard_image' })
    $refusals = @($errors | Where-Object { $_.command -eq 'paste_clipboard_image' })
    Check 'no IPC command failed but the one paste of text' ($unexpected.Count -eq 0 -and $refusals.Count -eq 1) ("failed: " + (($errors | ForEach-Object { $_.command }) -join ', '))
    Cdp shot 'shots/clipboard-final.png' | Out-Null
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
    # --- the person's clipboard, back
    & powershell.exe -NoProfile -ExecutionPolicy Bypass -File $backupScript -Action restore -Path $backupDir | Out-Null
    $verify = & powershell.exe -NoProfile -ExecutionPolicy Bypass -File $backupScript -Action verify -Path $backupDir
    Check 'your clipboard is back exactly as it was' ($LASTEXITCODE -eq 0) ($verify -join '; ')
}
$failed = @($results | Where-Object { -not $_.Ok })
Write-Host ''
Write-Host "$($results.Count - $failed.Count) of $($results.Count) checks passed"
if ($failed.Count -gt 0) { exit 1 }
