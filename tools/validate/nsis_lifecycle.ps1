# NSIS installer lifecycle, per-user, no elevation: install silently, inspect, launch the INSTALLED
# executable (own data folders, debug port), open a RAW region in it, close it with WM_CLOSE,
# uninstall silently, inspect what is left. The report is written next to the other validation
# output ($PfValidate\nsis-report.txt).
#
# Needs: the installer built (npm run tauri build), node, and the RAW sample inputs
# (node tools\validate\make_inputs.mjs). Run it on a machine where PhotoForge is not installed for
# real: it installs to, and removes, the per-user location %LOCALAPPDATA%\PhotoForge.
param([string]$Setup = '')
. "$PSScriptRoot\env.ps1"
if (-not $Setup) { $Setup = (Get-ChildItem (Join-Path $PfBundles 'nsis\*.exe') | Select-Object -First 1 -ExpandProperty FullName) }
$Report = Join-Path $PfValidate 'nsis-report.txt'
$lines = New-Object System.Collections.Generic.List[string]
function Say($text) { $lines.Add($text); Write-Output $text }

$userInstall = Join-Path $env:LOCALAPPDATA 'PhotoForge'
$uninstallKeys = @('HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall', 'HKLM:\Software\Microsoft\Windows\CurrentVersion\Uninstall', 'HKLM:\Software\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall')
function Find-Registered {
    foreach ($hive in $uninstallKeys) {
        Get-ChildItem $hive -ErrorAction SilentlyContinue | ForEach-Object {
            $p = Get-ItemProperty $_.PSPath
            if ($p.DisplayName -match 'PhotoForge') { [pscustomobject]@{ Hive = $hive; Name = $p.DisplayName; Version = $p.DisplayVersion; Location = $p.InstallLocation; Uninstall = $p.UninstallString } }
        }
    }
}
$inputs = Join-Path $PfValidate 'inputs'
if (-not (Test-Path (Join-Path $inputs 'portrait.dng'))) { throw "No RAW inputs in $inputs. Run: node tools\validate\make_inputs.mjs" }

Say "setup: $Setup ($([math]::Round((Get-Item $Setup).Length / 1MB, 1)) MB)"
Say "before: install dir exists = $(Test-Path $userInstall); registered = $(@(Find-Registered).Count)"

# 1. install
$sw = [Diagnostics.Stopwatch]::StartNew()
$proc = Start-Process -FilePath $Setup -ArgumentList '/S' -PassThru -Wait
Say "install: exit code $($proc.ExitCode) in $([math]::Round($sw.Elapsed.TotalSeconds, 1)) s"
$files = Get-ChildItem $userInstall -Recurse -File -ErrorAction SilentlyContinue
Say "installed: $($files.Count) files, $([math]::Round(($files | Measure-Object Length -Sum).Sum / 1MB, 1)) MB in $userInstall"
$files | ForEach-Object { Say ("  " + $_.FullName.Substring($userInstall.Length + 1) + "  " + $_.Length) }
@(Find-Registered) | ForEach-Object { Say "registered: $($_.Hive) | $($_.Name) $($_.Version) | $($_.Location) | $($_.Uninstall)" }
$exe = Join-Path $userInstall 'photoforge.exe'
Get-ChildItem "$env:APPDATA\Microsoft\Windows\Start Menu\Programs" -Recurse -Filter '*PhotoForge*' -ErrorAction SilentlyContinue |
    ForEach-Object { Say "start menu: $($_.FullName)" }

# 2. launch the installed executable and drive it
if (Test-Path $exe) {
    $out = & "$PSScriptRoot\launch.ps1" -Exe $exe
    $procId = [int](($out | Select-String 'PID (\d+)').Matches[0].Groups[1].Value)
    Start-Sleep -Seconds 8
    Push-Location $PfValidate
    node $PfCdp install | Out-Null
    node $PfCdp eval "window.__pf.nextOpen = '$(($inputs -replace '\\', '/'))/portrait.dng'; 1" | Out-Null
    node $PfCdp clickText Open | Out-Null; Start-Sleep -Seconds 4
    node $PfCdp clickText 'Choose Region' | Out-Null; Start-Sleep -Seconds 7
    node $PfCdp clickText 'Open region' | Out-Null; Start-Sleep -Seconds 12
    node $PfCdp shot 'shots/nsis-installed-region.png' | Out-Null
    $log = node $PfCdp log | Out-String
    $ok = ($log -match '"command": "open_raw_image"') -and ($log -match '"command": "render_layer_thumbnail"')
    Say "installed app: region opened = $ok; IPC errors in log = $(([regex]::Matches($log, '"error":')).Count)"
    Pop-Location
    $p = Get-Process -Id $procId -ErrorAction SilentlyContinue
    if ($p) {
        $null = $p.CloseMainWindow()
        for ($i = 0; $i -lt 20; $i++) { Start-Sleep -Milliseconds 500; $p.Refresh(); if ($p.HasExited) { break } }
        Say "installed app: WM_CLOSE exited = $($p.HasExited) after ~$([math]::Round($i * 0.5, 1)) s"
        if (-not $p.HasExited) { Stop-Process -Id $procId -Force }
    }
}

# 3. uninstall
$uninstaller = Join-Path $userInstall 'uninstall.exe'
if (Test-Path $uninstaller) {
    $sw.Restart()
    $proc = Start-Process -FilePath $uninstaller -ArgumentList '/S' -PassThru -Wait
    Start-Sleep -Seconds 3
    Say "uninstall: exit code $($proc.ExitCode) in $([math]::Round($sw.Elapsed.TotalSeconds, 1)) s"
} else {
    Say 'uninstall: no uninstall.exe in the install directory'
}
$left = Get-ChildItem $userInstall -Recurse -ErrorAction SilentlyContinue
Say "after: install dir exists = $(Test-Path $userInstall); files left = $(@($left | Where-Object { -not $_.PSIsContainer }).Count); registered = $(@(Find-Registered).Count)"
$lines | Set-Content $Report
