# Starts the release executable for a validation run, and prints "PID <n>".
#
#   * It gets its own LOCALAPPDATA (settings, recovery snapshots, caches), so a run never reads or
#     changes the settings of an installed PhotoForge;
#   * its own WebView2 profile;
#   * and a local-only remote-debugging port, which tools\validate\cdp.mjs speaks to. Nothing is
#     exposed beyond 127.0.0.1, and only for the process started here.
param(
    [string]$Exe = '',
    [int]$Port = $(if ($env:PF_CDP_PORT) { [int]$env:PF_CDP_PORT } else { 9333 })
)
. "$PSScriptRoot\env.ps1"
if (-not $Exe) { $Exe = $PfExe }
if (-not (Test-Path $Exe)) { throw "No executable at $Exe. Build one: npm run build, then cargo build --release in src-tauri (or npm run tauri build)." }
$data = Join-Path $PfValidate 'localappdata'
$profileDir = Join-Path $PfValidate 'webview'
New-Item -ItemType Directory -Force $data, $profileDir | Out-Null
# The RAW scenarios go through the region chooser, which is only offered for a file that does not
# fit the memory budget. A new profile's automatic budget on a roomy machine fits the 22-megapixel
# sample whole, so (unless the profile already has one) this writes the file the Memory settings
# write: a manual budget, 512 MiB by default. PF_BUDGET_BYTES changes it; 0 leaves the profile alone.
$settingsDir = Join-Path $data 'PhotoForge\settings'
$resourcesFile = Join-Path $settingsDir 'resources.json'
if ($env:PF_BUDGET_BYTES -ne '0' -and -not (Test-Path $resourcesFile)) {
    $budgetBytes = if ($env:PF_BUDGET_BYTES) { [long]$env:PF_BUDGET_BYTES } else { 536870912 }
    New-Item -ItemType Directory -Force $settingsDir | Out-Null
    $json = '{"version":1,"budget":{"mode":"manual","bytes":' + $budgetBytes + '}}'
    [System.IO.File]::WriteAllText($resourcesFile, $json, (New-Object System.Text.UTF8Encoding $false))
}
# The child inherits these; they are put back at once so the rest of the calling script (an
# installer run, say) is not affected by them.
$saved = @{}
foreach ($name in 'LOCALAPPDATA', 'WEBVIEW2_USER_DATA_FOLDER', 'WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS') {
    $saved[$name] = [Environment]::GetEnvironmentVariable($name, 'Process')
}
try {
    $env:LOCALAPPDATA = $data
    $env:WEBVIEW2_USER_DATA_FOLDER = $profileDir
    $env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS = "--remote-debugging-port=$Port --remote-allow-origins=*"
    $process = Start-Process -FilePath $Exe -PassThru
} finally {
    foreach ($name in $saved.Keys) { [Environment]::SetEnvironmentVariable($name, $saved[$name], 'Process') }
}
Write-Output "PID $($process.Id)"
