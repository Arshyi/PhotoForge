# Guarded, staged all-users MSI validation. Preflight is read-only and is the default.
# Install/Uninstall request UAC consent from a person; this script never answers it.
# Launch the exact verified executable separately with launch.ps1's isolated data,
# and close that test window before requesting Uninstall.
[CmdletBinding()]
param(
    [ValidateSet('Preflight', 'Install', 'Verify', 'Uninstall')][string]$Action = 'Preflight',
    [string]$Msi = '',
    [string]$RunDirectory = '',
    [ValidateRange(1, 60)][int]$ElevationTimeoutSeconds = 60
)
$ErrorActionPreference = 'Stop'
if (-not [Environment]::Is64BitProcess) { throw 'Run this harness in 64-bit PowerShell.' }
$repoPath = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..\..')).ProviderPath
$runRoot = [IO.Path]::GetFullPath((Join-Path $repoPath '.work\msi-lifecycle'))
$machinePath = Join-Path ([Environment]::GetFolderPath('ProgramFiles')) 'PhotoForge'
$machineExe = Join-Path $machinePath 'photoforge.exe'
$machineKey = 'HKLM:\Software\photoforge\PhotoForge'
$userKey = 'HKCU:\Software\photoforge\PhotoForge'
$commonDesktop = [Environment]::GetFolderPath('CommonDesktopDirectory')
$statePath = $null
$state = $null

function Assert-Run($condition, [string]$message) { if (-not $condition) { throw $message } }
function Full-Path([string]$path) { [IO.Path]::GetFullPath($path).TrimEnd('\') }
function Assert-NoLinks([string]$path) {
    $cursor = Full-Path $path
    while ($cursor) {
        if (Test-Path -LiteralPath $cursor) {
            $item = Get-Item -LiteralPath $cursor -Force
            Assert-Run (($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -eq 0) "Refusing a linked path: $cursor"
        }
        $parent = [IO.Path]::GetDirectoryName($cursor)
        if ($parent -eq $cursor) { break }
        $cursor = $parent
    }
}
function Registered-Products {
    foreach ($hive in 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall',
        'HKLM:\Software\Microsoft\Windows\CurrentVersion\Uninstall',
        'HKLM:\Software\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall') {
        if (-not (Test-Path -LiteralPath $hive)) { continue }
        foreach ($key in Get-ChildItem -LiteralPath $hive) {
            $entry = Get-ItemProperty -LiteralPath $key.PSPath
            if ($entry.DisplayName -match '^PhotoForge(?: |$)') {
                [pscustomobject]@{ key=$key.PSChildName; hive=$hive; version=$entry.DisplayVersion }
            }
        }
    }
}
function Assert-NoApp {
    Assert-Run (@(Get-Process -Name photoforge -ErrorAction SilentlyContinue).Count -eq 0) 'Close PhotoForge yourself before this action; no other session will be closed.'
}
function User-RegistrySnapshot {
    if (-not (Test-Path -LiteralPath $userKey)) { return 'absent' }
    $key = Get-Item -LiteralPath $userKey
    try {
        $entries = @(foreach ($name in ($key.GetValueNames() | Sort-Object)) {
            [ordered]@{ name=$name; kind=[string]$key.GetValueKind($name);
                value=$key.GetValue($name, $null, [Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames) }
        })
        ConvertTo-Json -InputObject $entries -Compress -Depth 8
    } finally { $key.Close() }
}
function Assert-Pristine {
    Assert-NoApp
    Assert-Run (@(Registered-Products).Count -eq 0) 'PhotoForge is already registered; this harness will not upgrade or remove it.'
    Assert-Run (-not (Test-Path -LiteralPath $machinePath)) "Existing directory will not be replaced: $machinePath"
    Assert-Run (-not (Test-Path -LiteralPath $machineKey)) 'Existing machine product state will not be replaced.'
    Assert-Run (-not (Test-Path -LiteralPath (Join-Path $env:LOCALAPPDATA 'PhotoForge\photoforge.exe'))) 'An unregistered per-user PhotoForge installation exists.'
    foreach ($folder in @([Environment]::GetFolderPath('Desktop'), $commonDesktop,
        [Environment]::GetFolderPath('Programs'), [Environment]::GetFolderPath('CommonPrograms'))) {
        Assert-Run (-not (Test-Path -LiteralPath (Join-Path $folder 'PhotoForge.lnk'))) "Existing PhotoForge shortcut in $folder will not be replaced."
        Assert-Run (-not (Test-Path -LiteralPath (Join-Path $folder 'PhotoForge'))) "Existing PhotoForge shortcut directory in $folder will not be replaced."
    }
    # A stale NSIS InstallDir is allowed, but never an executable at that directory.
    if (Test-Path -LiteralPath $userKey) {
        $key = Get-Item -LiteralPath $userKey
        try {
            foreach ($name in '', 'InstallDir') {
                $directory = $key.GetValue($name)
                if ($directory -is [string] -and $directory) {
                    Assert-Run (-not (Test-Path -LiteralPath (Join-Path $directory 'photoforge.exe'))) 'An unregistered executable exists at the retained per-user path.'
                }
            }
        } finally { $key.Close() }
    }
    Assert-Run (Test-Path -LiteralPath $commonDesktop -PathType Container) 'The shared Desktop must exist before this test.'
    Assert-NoLinks $machinePath
}
function Save-State {
    $state.updatedUtc = [DateTime]::UtcNow.ToString('o')
    $state | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $statePath -Encoding UTF8
}
function Emit-State { $state | ConvertTo-Json -Depth 8 }
function Assert-Installed {
    Assert-NoApp
    $products = @(Registered-Products)
    Assert-Run ($products.Count -eq 1 -and $products[0].key -ieq $state.productCode -and
        $products[0].hive -eq 'HKLM:\Software\Microsoft\Windows\CurrentVersion\Uninstall' -and
        $products[0].version -eq $state.version) 'Registration is not the exact machine product installed by this run.'
    Assert-NoLinks $machinePath
    Assert-Run (Test-Path -LiteralPath $machineExe -PathType Leaf) 'No executable at the expected Program Files directory.'
    $registeredPath = (Get-ItemProperty -LiteralPath $machineKey -Name InstallDir).InstallDir
    Assert-Run ((Full-Path $registeredPath) -ieq (Full-Path $machinePath)) 'Machine InstallDir is not the expected default.'
    Assert-Run ((User-RegistrySnapshot) -ceq $state.userRegistryBefore) 'Per-user NSIS product state changed.'
    Assert-Run (Test-Path -LiteralPath $commonDesktop -PathType Container) 'The shared Desktop directory disappeared.'
    foreach ($resource in 'THIRD_PARTY_NOTICES.md', 'licenses\Tauri-MIT.txt') {
        Assert-Run (Test-Path -LiteralPath (Join-Path $machinePath $resource) -PathType Leaf) "Installed resource is missing: $resource"
    }
    $hash = (Get-FileHash -LiteralPath $machineExe -Algorithm SHA256).Hash
    if ($state.installedExeHash) { Assert-Run ($hash -eq $state.installedExeHash) 'Executable changed since verification; refusing automatic removal.' }
    $state.installedExeHash = $hash
}
function Assert-Uninstalled {
    Assert-NoApp
    Assert-Run (@(Registered-Products).Count -eq 0) 'PhotoForge registration remains after uninstall.'
    Assert-Run (-not (Test-Path -LiteralPath $machinePath)) 'Files remain; no directory will be force-deleted.'
    Assert-Run (-not (Test-Path -LiteralPath $machineKey)) 'Machine product state remains after uninstall.'
    Assert-Run ((User-RegistrySnapshot) -ceq $state.userRegistryBefore) 'Uninstall changed per-user NSIS state.'
    Assert-Run (Test-Path -LiteralPath $commonDesktop -PathType Container) 'Uninstall removed the shared Desktop directory.'
    Assert-Run (-not (Test-Path -LiteralPath (Join-Path $commonDesktop 'PhotoForge.lnk'))) 'Machine Desktop shortcut remains.'
    Assert-Run (-not (Test-Path -LiteralPath (Join-Path ([Environment]::GetFolderPath('CommonPrograms')) 'PhotoForge'))) 'Machine Start Menu directory remains.'
    $state.uninstallVerified = $true
    $state.phase = 'uninstalled-verified'
}
function Elevate-Msi([string]$arguments, [string]$operation) {
    # A timeout does not kill msiexec or assume it failed. Inspect before any next action.
    $state.phase = "$operation-requested"
    Save-State
    try {
        $process = Start-Process -FilePath (Join-Path $env:SystemRoot 'System32\msiexec.exe') -ArgumentList $arguments -Verb RunAs -WindowStyle Hidden -PassThru
    } catch {
        $state.phase = "$operation-not-started"
        $state.lastError = $_.Exception.Message
        Save-State
        throw
    }
    $state.msiexecPid = $process.Id
    Save-State
    if (-not $process.WaitForExit($ElevationTimeoutSeconds * 1000)) {
        $state.phase = "$operation-pending"
        Save-State
        throw "msiexec is still running. Do not repeat $operation or close its consent prompt. Inspect after it finishes: $RunDirectory"
    }
    $state.lastExitCode = $process.ExitCode
    $state.msiexecPid = $null
    $state.phase = "$operation-finished"
    Save-State
    Assert-Run ($process.ExitCode -in 0, 3010) "MSI $operation failed with exit code $($process.ExitCode); logs retained in $RunDirectory"
    $state.restartRequired = $process.ExitCode -eq 3010
}

if ($Action -in 'Verify', 'Uninstall') {
    Assert-Run ([bool]$RunDirectory) 'Specify the RunDirectory returned by Install.'
    $RunDirectory = Full-Path $RunDirectory
    Assert-Run ($RunDirectory.StartsWith($runRoot + '\', [StringComparison]::OrdinalIgnoreCase) -and
        [IO.Path]::GetFileName($RunDirectory) -match '^run-[0-9a-f]{32}$') 'RunDirectory must identify a harness-owned run under .work\msi-lifecycle.'
    Assert-NoLinks $RunDirectory
    $statePath = Join-Path $RunDirectory 'state.json'
    $state = Get-Content -LiteralPath $statePath -Raw | ConvertFrom-Json
    Assert-Run ($state.schema -eq 1 -and $state.runDirectory -ieq $RunDirectory -and $state.installDirectory -ieq $machinePath) 'Invalid or relocated run state.'
    if ($Msi) { Assert-Run ((Resolve-Path -LiteralPath $Msi).ProviderPath -ieq $state.msi) 'MSI differs from this run.' }
    $Msi = $state.msi
    if ($state.msiexecPid) {
        Assert-Run (-not (Get-Process -Id $state.msiexecPid -ErrorAction SilentlyContinue)) 'The previous msiexec is still running; wait for it to finish.'
    }
} else {
    Assert-Run ([bool]$Msi) 'An explicit MSI path is required; no arbitrary first package will be selected.'
    Assert-Run (-not $RunDirectory) 'Install creates a unique run directory; Preflight creates none.'
}
$Msi = (Resolve-Path -LiteralPath $Msi).ProviderPath
$expectedVersion = if ($state) { $state.version } else { (Get-Content -LiteralPath (Join-Path $repoPath 'src-tauri\tauri.conf.json') -Raw | ConvertFrom-Json).version }
$policy = & (Join-Path $PSScriptRoot 'check_msi.ps1') -Msi $Msi -ExpectedVersion $expectedVersion | ConvertFrom-Json
Assert-Run ($policy.status -eq 'pass' -and $policy.productCode -match '^\{[0-9A-Fa-f-]{36}\}$') 'MSI policy inspection failed.'
$msiHash = (Get-FileHash -LiteralPath $Msi -Algorithm SHA256).Hash
if ($state) {
    Assert-Run ($msiHash -eq $state.msiHash -and $policy.productCode -ieq $state.productCode) 'MSI identity or contents changed since Install.'
} else {
    Assert-Pristine
    if ($Action -eq 'Preflight') {
        [pscustomobject]@{ status='preflight-pass'; msi=$Msi; productCode=$policy.productCode;
            installDirectory=$machinePath; writesPerformed=$false; nativeLifecycleVerified=$false } | ConvertTo-Json
        return
    }
    Assert-NoLinks $runRoot
    $RunDirectory = Join-Path $runRoot ('run-' + [Guid]::NewGuid().ToString('N'))
    $null = New-Item -ItemType Directory -Path $RunDirectory
    $statePath = Join-Path $RunDirectory 'state.json'
    $state = [pscustomobject][ordered]@{
        schema=1; runDirectory=$RunDirectory; msi=$Msi; msiHash=$msiHash;
        productCode=$policy.productCode; version=$policy.version; installDirectory=$machinePath;
        userRegistryBefore=(User-RegistrySnapshot); installedExeHash=$null;
        phase='preflight-pass'; createdUtc=[DateTime]::UtcNow.ToString('o'); updatedUtc=$null;
        msiexecPid=$null; lastExitCode=$null; lastError=$null; restartRequired=$false;
        installedVerified=$false; launchVerified=$false; uninstallVerified=$false;
        nativeLifecycleVerified=$false
    }
    Save-State
    Write-Output "RunDirectory: $RunDirectory"
}
try {
    switch ($Action) {
        'Install' {
            $logPath = Join-Path $RunDirectory 'install.log'
            $arguments = '/i "{0}" /qn /norestart AUTOLAUNCHAPP=0 WIXUI_EXITDIALOGOPTIONALCHECKBOX=0 /l*v "{1}"' -f $Msi, $logPath
            Elevate-Msi $arguments 'install'
            Assert-Installed
            $state.installedVerified = $true
            $state.phase = 'installed-verified'
        }
        'Verify' {
            Assert-Run ($state.phase -notin 'uninstalled-verified', 'install-not-started') 'This run did not install a currently owned product.'
            if ($state.installedVerified -and $state.phase -in 'uninstall-pending', 'uninstall-finished', 'uninstall-requested') {
                Assert-Uninstalled
            } else {
                Assert-Installed
                $state.installedVerified = $true
                $state.phase = 'installed-verified'
            }
            $state.msiexecPid = $null
        }
        'Uninstall' {
            Assert-Run ($state.installedVerified -and $state.phase -in 'installed-verified', 'uninstall-not-started') 'A verified owned installation is required; do not repeat a pending/finished uninstall.'
            Assert-Installed
            $logPath = Join-Path $RunDirectory ('uninstall-' + [Guid]::NewGuid().ToString('N') + '.log')
            Elevate-Msi ('/x {0} /qn /norestart /l*v "{1}"' -f $state.productCode, $logPath) 'uninstall'
            Assert-Uninstalled
        }
    }
    Save-State
    Emit-State
} catch {
    $state.lastError = $_.Exception.Message
    Save-State
    throw
}
