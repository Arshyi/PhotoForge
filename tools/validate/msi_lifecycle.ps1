# MSI installer lifecycle. The package is per-machine, so install and uninstall run elevated (one
# UAC elevation each, which a person has to answer); the launch and the drive run as the ordinary
# user, from the installed executable. The report is $PfValidate\msi-report.txt.
#
# Known (docs\phase-15-results.md): the package is registered per machine but installs its files into
# the installing user's profile (%LOCALAPPDATA%\PhotoForge).
param(
    [string]$Msi = '',
    [int]$ElevationTimeoutSeconds = 180
)
. "$PSScriptRoot\env.ps1"
if (-not $Msi) { $Msi = (Get-ChildItem (Join-Path $PfBundles 'msi\*.msi') | Select-Object -First 1 -ExpandProperty FullName) }
$Report = Join-Path $PfValidate 'msi-report.txt'
$lines = New-Object System.Collections.Generic.List[string]
function Say($text) { $lines.Add($text); Write-Output $text }

$uninstallKeys = @('HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall', 'HKLM:\Software\Microsoft\Windows\CurrentVersion\Uninstall', 'HKLM:\Software\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall')
function Find-Registered {
    foreach ($hive in $uninstallKeys) {
        Get-ChildItem $hive -ErrorAction SilentlyContinue | ForEach-Object {
            $p = Get-ItemProperty $_.PSPath
            if ($p.DisplayName -match 'PhotoForge') { [pscustomobject]@{ Hive = $hive; Name = $p.DisplayName; Version = $p.DisplayVersion; Location = $p.InstallLocation } }
        }
    }
}
# Say() inside a function would also be captured as its output, so the exit code is returned
# through a script-scope variable instead.
function Run-Elevated($arguments, $label) {
    $sw = [Diagnostics.Stopwatch]::StartNew()
    $script:LastExit = $null
    $proc = Start-Process -FilePath 'msiexec.exe' -ArgumentList $arguments -Verb RunAs -PassThru
    if (-not $proc.WaitForExit($ElevationTimeoutSeconds * 1000)) {
        Say "${label}: still waiting after $ElevationTimeoutSeconds s (a UAC prompt may be waiting on the desktop)"
        return
    }
    $script:LastExit = $proc.ExitCode
    Say "${label}: exit code $($proc.ExitCode) in $([math]::Round($sw.Elapsed.TotalSeconds, 1)) s"
}
$inputs = Join-Path $PfValidate 'inputs'
if (-not (Test-Path (Join-Path $inputs 'portrait.dng'))) { throw "No RAW inputs in $inputs. Run: node tools\validate\make_inputs.mjs" }

Say "package: $Msi ($([math]::Round((Get-Item $Msi).Length / 1MB, 1)) MB)"
Say "before: registered = $(@(Find-Registered).Count)"
$installLog = Join-Path $PfValidate 'msi-install.log'
$removeLog = Join-Path $PfValidate 'msi-remove.log'
Remove-Item $installLog, $removeLog -ErrorAction SilentlyContinue

# 1. install (elevated)
Run-Elevated "/i `"$Msi`" /qn /norestart /l*v `"$installLog`"" 'install'
if ($null -eq $script:LastExit) { $lines | Set-Content $Report; return }
@(Find-Registered) | ForEach-Object { Say "registered: $($_.Hive) | $($_.Name) $($_.Version) | $($_.Location)" }
$logText = if (Test-Path $installLog) { Get-Content $installLog -Raw } else { '' }
$dir = $null
$m = [regex]::Match($logText, 'Property\(S\): (?:INSTALLDIR|APPLICATIONFOLDER) = (.+)')
if ($m.Success) { $dir = $m.Groups[1].Value.Trim() }
if (-not $dir) {
    foreach ($candidate in @("$env:LOCALAPPDATA\PhotoForge", 'C:\Program Files\PhotoForge')) { if (Test-Path (Join-Path $candidate 'photoforge.exe')) { $dir = $candidate } }
}
Say "install directory: $dir"
$exe = if ($dir) { Join-Path $dir 'photoforge.exe' } else { $null }
if ($exe -and (Test-Path $exe)) {
    $files = Get-ChildItem $dir -Recurse -File
    Say "installed: $($files.Count) files, $([math]::Round(($files | Measure-Object Length -Sum).Sum / 1MB, 1)) MB"
    $files | ForEach-Object { Say ("  " + $_.FullName.Substring($dir.Length).TrimStart('\') + "  " + $_.Length) }

    # 2. launch the installed executable as the ordinary user, own data, and drive a RAW region open
    $out = & "$PSScriptRoot\launch.ps1" -Exe $exe
    $procId = [int](($out | Select-String 'PID (\d+)').Matches[0].Groups[1].Value)
    Start-Sleep -Seconds 8
    Push-Location $PfValidate
    node $PfCdp install | Out-Null
    node $PfCdp eval "window.__pf.nextOpen = '$(($inputs -replace '\\', '/'))/portrait.dng'; 1" | Out-Null
    node $PfCdp clickText Open | Out-Null; Start-Sleep -Seconds 4
    node $PfCdp clickText 'Choose Region' | Out-Null; Start-Sleep -Seconds 7
    node $PfCdp clickText 'Open region' | Out-Null; Start-Sleep -Seconds 12
    node $PfCdp shot 'shots/msi-installed-region.png' | Out-Null
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
} else {
    Say 'no installed exe was found; skipping the launch'
}

# 3. uninstall (elevated)
Run-Elevated "/x `"$Msi`" /qn /norestart /l*v `"$removeLog`"" 'uninstall'
Start-Sleep -Seconds 2
$left = if ($dir) { Get-ChildItem $dir -Recurse -ErrorAction SilentlyContinue } else { @() }
Say "after: install dir exists = $(if ($dir) { Test-Path $dir } else { 'n/a' }); files left = $(@($left | Where-Object { -not $_.PSIsContainer }).Count); registered = $(@(Find-Registered).Count)"
$lines | Set-Content $Report
