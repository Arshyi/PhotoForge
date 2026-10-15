# Guarded payload/registration validation, NOT native GUI or shortcut acceptance.
# Preflight is read-only. Install and Uninstall must be requested separately.
# Use a disposable ordinary-user test account with no existing PhotoForge state.
# No app launch, CDP, clipboard access, elevation, process kill or cleanup deletion.
[CmdletBinding()]
param(
    [ValidateSet('Preflight', 'Install', 'Verify', 'Uninstall')][string]$Action = 'Preflight',
    [string]$Setup = '',
    [string]$SetupSha256 = '',
    [string]$ReferenceExe = '',
    [string]$RunDirectory = '',
    [ValidateRange(1, 60)][int]$TimeoutSeconds = 60
)
$ErrorActionPreference = 'Stop'
if (-not [Environment]::Is64BitProcess) { throw 'Run this harness in 64-bit PowerShell.' }
$repoPath = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..\..')).ProviderPath
$runRoot = [IO.Path]::GetFullPath((Join-Path $repoPath '.work\nsis-lifecycle'))
$userKey = 'HKCU:\Software\photoforge\PhotoForge'
$machineKey = 'HKLM:\Software\photoforge\PhotoForge'
$userUninstallHive = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall'
$runKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run'
$state = $null
$statePath = $null

function Assert-Run($condition, [string]$message) { if (-not $condition) { throw $message } }
function Full-Path([string]$path) {
    Assert-Run ($path -match '^[A-Za-z]:\\' -and $path -notmatch '["\x00-\x1f]') 'Use an absolute local drive path without quotes or control characters.'
    [IO.Path]::GetFullPath($path).TrimEnd('\')
}
function Assert-NoLinks([string]$path) {
    $cursor = Full-Path $path
    while ($cursor) {
        # Test-Path can report false for a dangling link or an inaccessible path.
        # Inspect the entry itself; only a genuinely missing entry may be skipped.
        $item = $null
        try { $item = Get-Item -LiteralPath $cursor -Force -ErrorAction Stop }
        catch {
            if ($_.Exception -isnot [Management.Automation.ItemNotFoundException]) { throw }
        }
        if ($item) {
            Assert-Run (($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -eq 0) "Refusing a linked path: $cursor"
        }
        $parent = [IO.Path]::GetDirectoryName($cursor)
        if ($parent -eq $cursor) { break }
        $cursor = $parent
    }
}
function Assert-OrdinaryUser {
    $identity = [Security.Principal.WindowsIdentity]::GetCurrent()
    try {
        $principal = New-Object Security.Principal.WindowsPrincipal $identity
        Assert-Run (-not $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) 'Use an ordinary, non-elevated PowerShell session.'
    } finally { $identity.Dispose() }
}
function Assert-NoApp {
    Assert-Run (@(Get-Process -Name photoforge -ErrorAction SilentlyContinue).Count -eq 0) 'Close PhotoForge yourself first; no session will be closed by this harness.'
}
function Assert-WebView2Present {
    # Match the pinned x64 NSIS prerequisite lookup. A version registration is only
    # a prerequisite guard, not proof of runtime health or availability to other users.
    $guid = '{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}'
    foreach ($key in "HKLM:\Software\WOW6432Node\Microsoft\EdgeUpdate\Clients\$guid",
        "HKCU:\Software\Microsoft\EdgeUpdate\Clients\$guid") {
        if (-not (Test-Path -LiteralPath $key)) { continue }
        $version = [version]'0.0.0.0'
        $value = (Get-ItemProperty -LiteralPath $key -Name pv -ErrorAction Stop).pv
        if ([version]::TryParse([string]$value, [ref]$version) -and $version -gt [version]'0.0.0.0') { return }
    }
    throw 'WebView2 prerequisite registration is missing/invalid; this harness will not download or provision it.'
}
function Registered-Products {
    foreach ($hive in $userUninstallHive, 'HKCU:\Software\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall',
        'HKLM:\Software\Microsoft\Windows\CurrentVersion\Uninstall', 'HKLM:\Software\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall') {
        if (-not (Test-Path -LiteralPath $hive)) { continue }
        foreach ($key in Get-ChildItem -LiteralPath $hive) {
            $entry = Get-ItemProperty -LiteralPath $key.PSPath
            if ($key.PSChildName -ieq 'PhotoForge' -or $entry.DisplayName -match '^PhotoForge(?: |$)') {
                [pscustomobject]@{ key=$key.PSChildName; hive=$hive; version=$entry.DisplayVersion;
                    publisher=$entry.Publisher; location=$entry.InstallLocation; uninstall=$entry.UninstallString;
                    mainBinary=$entry.MainBinaryName }
            }
        }
    }
}
function Shortcut-Paths {
    foreach ($folder in @([Environment]::GetFolderPath('Desktop'), [Environment]::GetFolderPath('CommonDesktopDirectory'),
        [Environment]::GetFolderPath('Programs'), [Environment]::GetFolderPath('CommonPrograms'))) {
        Join-Path $folder 'PhotoForge.lnk'
    }
    # A Desktop project directory called photoforge is NOT an installer directory.
    foreach ($folder in @([Environment]::GetFolderPath('Programs'), [Environment]::GetFolderPath('CommonPrograms'))) {
        Join-Path $folder 'PhotoForge'
    }
}
function Assert-ShellUntouched {
    foreach ($path in Shortcut-Paths) {
        Assert-Run (-not (Test-Path -LiteralPath $path)) "Existing/unexpected PhotoForge shortcut state: $path"
    }
    if (Test-Path -LiteralPath $runKey) {
        $key = Get-Item -LiteralPath $runKey
        try { Assert-Run ($key.GetValueNames() -inotcontains 'PhotoForge') 'Existing/unexpected PhotoForge autostart entry; it will not be removed.' }
        finally { $key.Close() }
    }
}
function Assert-Pristine {
    Assert-NoApp
    Assert-Run (@(Registered-Products).Count -eq 0) 'PhotoForge is already registered; this harness will not upgrade or remove it.'
    foreach ($key in $userKey, $machineKey, 'HKCU:\Software\WOW6432Node\photoforge\PhotoForge', 'HKLM:\Software\WOW6432Node\photoforge\PhotoForge') {
        Assert-Run (-not (Test-Path -LiteralPath $key)) "Existing product registry state (even stale) will not be replaced: $key"
    }
    foreach ($path in (Join-Path $env:LOCALAPPDATA 'PhotoForge'), (Join-Path ([Environment]::GetFolderPath('ProgramFiles')) 'PhotoForge'),
        (Join-Path ([Environment]::GetFolderPath('ProgramFilesX86')) 'PhotoForge')) {
        Assert-NoLinks $path
        Assert-Run (-not (Test-Path -LiteralPath $path)) "Existing PhotoForge directory will not be replaced: $path"
    }
    Assert-ShellUntouched
}
function Read-ExecutionLevel([string]$path) {
    # Load PE resources as DATA ONLY: never load/execute the candidate's code.
    if (-not ('PhotoForge.Validation.PeManifest' -as [type])) {
        Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
using System.Text;
namespace PhotoForge.Validation {
    public static class PeManifest {
        [DllImport("kernel32.dll", CharSet=CharSet.Unicode, SetLastError=true)]
        static extern IntPtr LoadLibraryEx(string file, IntPtr reserved, uint flags);
        [DllImport("kernel32.dll", SetLastError=true)]
        static extern IntPtr FindResource(IntPtr module, IntPtr name, IntPtr type);
        [DllImport("kernel32.dll", SetLastError=true)]
        static extern uint SizeofResource(IntPtr module, IntPtr resource);
        [DllImport("kernel32.dll", SetLastError=true)]
        static extern IntPtr LoadResource(IntPtr module, IntPtr resource);
        [DllImport("kernel32.dll")]
        static extern IntPtr LockResource(IntPtr resource);
        [DllImport("kernel32.dll")]
        static extern bool FreeLibrary(IntPtr module);
        public static string Read(string path) {
            IntPtr module = LoadLibraryEx(path, IntPtr.Zero, 0x22);
            if (module == IntPtr.Zero) throw new System.ComponentModel.Win32Exception();
            try {
                IntPtr resource = FindResource(module, new IntPtr(1), new IntPtr(24));
                uint size = SizeofResource(module, resource);
                if (size == 0 || size > 1048576) throw new InvalidOperationException("Missing/oversized PE manifest.");
                IntPtr data = LockResource(LoadResource(module, resource));
                if (data == IntPtr.Zero) throw new System.ComponentModel.Win32Exception();
                byte[] bytes = new byte[(int)size];
                Marshal.Copy(data, bytes, 0, bytes.Length);
                return (bytes.Length > 1 && bytes[0] == 255 && bytes[1] == 254 ? Encoding.Unicode : Encoding.UTF8).GetString(bytes).TrimStart('\uFEFF').TrimEnd('\0');
            } finally { FreeLibrary(module); }
        }
    }
}
'@
    }
    $settings = New-Object Xml.XmlReaderSettings
    $settings.DtdProcessing = [Xml.DtdProcessing]::Prohibit
    $settings.XmlResolver = $null
    $reader = [Xml.XmlReader]::Create((New-Object IO.StringReader ([PhotoForge.Validation.PeManifest]::Read($path))), $settings)
    try {
        $document = New-Object Xml.XmlDocument
        $document.XmlResolver = $null
        $document.Load($reader)
        $levels = $document.SelectNodes('//*[local-name()="requestedExecutionLevel"]')
        Assert-Run ($levels.Count -eq 1 -and $levels[0].GetAttribute('uiAccess') -eq 'false') 'Unexpected/missing PE execution manifest.'
        $levels[0].GetAttribute('level')
    } finally { $reader.Dispose() }
}
function Assert-Package([string]$path, [string]$hash, [string]$version) {
    Assert-Run ($hash -match '^[0-9a-fA-F]{64}$') 'Pin the explicit installer with -SetupSha256 (64 hexadecimal characters).'
    Assert-NoLinks $path
    Assert-Run (Test-Path -LiteralPath $path -PathType Leaf) 'The pinned installer is missing.'
    Assert-Run ((Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash -ieq $hash) 'Installer hash changed or does not match the pinned candidate.'
    $info = (Get-Item -LiteralPath $path).VersionInfo
    Assert-Run ($info.ProductName -eq 'PhotoForge' -and $info.ProductVersion -eq $version -and $info.FileVersion -eq $version) 'Installer identity/version differs from the candidate.'
    Assert-Run ((Read-ExecutionLevel $path) -eq 'asInvoker') 'The NSIS candidate must be asInvoker; this harness never requests elevation.'
}
function Bundle-TagFromStream([IO.Stream]$stream) {
    # Bounded streaming inspection; no executable loading and no whole-file text copy.
    $prefix = '__TAURI_BUNDLE_TYPE_VAR_'
    $span = $prefix.Length + 3
    $buffer = New-Object byte[] 65536
    $tail = ''; $tags = @()
    while (($count = $stream.Read($buffer, 0, $buffer.Length)) -gt 0) {
        $chunk = $tail + [Text.Encoding]::ASCII.GetString($buffer, 0, $count)
        $offset = $chunk.IndexOf($prefix, [StringComparison]::Ordinal)
        while ($offset -ge 0 -and $offset + $span -le $chunk.Length) {
            $tags += $chunk.Substring($offset + $prefix.Length, 3)
            Assert-Run ($tags.Count -le 1) 'Reference has ambiguous Tauri bundle markers.'
            $offset = $chunk.IndexOf($prefix, $offset + $span, [StringComparison]::Ordinal)
        }
        $tail = $chunk.Substring([Math]::Max(0, $chunk.Length - ($span - 1)))
    }
    Assert-Run ($tags.Count -eq 1) 'Reference lacks a unique Tauri bundle marker.'
    $tags[0]
}
function Read-BundleTag([string]$path) {
    $stream = [IO.File]::OpenRead($path)
    try { Bundle-TagFromStream $stream } finally { $stream.Dispose() }
}
function Product-Snapshot {
    Assert-Run (Test-Path -LiteralPath $userKey) 'Owned per-user product state is missing.'
    $key = Get-Item -LiteralPath $userKey
    try {
        Assert-Run (@($key.GetSubKeyNames()).Count -eq 0) 'Unexpected product registry subkeys; refusing removal.'
        Assert-Run ($key.GetValue('') -ieq $state.installDirectory) 'Product path was redirected away from this run.'
        $values = @(foreach ($name in ($key.GetValueNames() | Sort-Object)) {
            Assert-Run ($name -in '', 'Installer Language') 'Unexpected product registry value; refusing removal.'
            [ordered]@{ name=$name; kind=[string]$key.GetValueKind($name); value=$key.GetValue($name) }
        })
        ConvertTo-Json -InputObject $values -Depth 4 -Compress
    } finally { $key.Close() }
}
function Assert-OwnedLayout {
    Assert-NoLinks $state.installDirectory
    Assert-Run (Test-Path -LiteralPath $state.installDirectory -PathType Container) 'Owned installation directory is missing.'
    $rootItems = @(Get-ChildItem -LiteralPath $state.installDirectory -Force)
    Assert-Run ($rootItems.Count -eq 4) 'Unexpected installed files/directories; nothing will be force-deleted.'
    foreach ($item in $rootItems) {
        Assert-NoLinks $item.FullName
        Assert-Run (($item.PSIsContainer -and $item.Name -ieq 'licenses') -or
            (-not $item.PSIsContainer -and $item.Name -iin @('photoforge.exe', 'uninstall.exe', 'THIRD_PARTY_NOTICES.md'))) 'Unexpected installed entry; refusing removal.'
    }
    $licenseItems = @(Get-ChildItem -LiteralPath (Join-Path $state.installDirectory 'licenses') -Force)
    Assert-Run ($licenseItems.Count -eq 1 -and -not $licenseItems[0].PSIsContainer -and $licenseItems[0].Name -ieq 'Tauri-MIT.txt') 'Unexpected license directory contents; refusing removal.'
    Assert-NoLinks $licenseItems[0].FullName
}
function Assert-Installed {
    Assert-NoApp
    Assert-ShellUntouched
    $products = @(Registered-Products)
    $uninstaller = Join-Path $state.installDirectory 'uninstall.exe'
    Assert-Run ($products.Count -eq 1 -and $products[0].hive -eq $userUninstallHive -and $products[0].key -ieq 'PhotoForge' -and
        $products[0].version -eq $state.version -and $products[0].publisher -eq 'photoforge' -and
        $products[0].mainBinary -eq 'photoforge.exe' -and $products[0].location -ieq ('"' + $state.installDirectory + '"') -and
        $products[0].uninstall -ieq ('"' + $uninstaller + '"')) 'Registration is not the exact per-user installation owned by this run.'
    Assert-Run (-not (Test-Path -LiteralPath $machineKey)) 'Unexpected machine product state; refusing removal.'
    Assert-OwnedLayout
    foreach ($file in $state.payloadHashes.PSObject.Properties) {
        Assert-Run ((Get-FileHash -LiteralPath (Join-Path $state.installDirectory $file.Name) -Algorithm SHA256).Hash -ieq $file.Value) "Installed payload changed/mismatched: $($file.Name)"
    }
    $hash = (Get-FileHash -LiteralPath $uninstaller -Algorithm SHA256).Hash
    if ($state.uninstallerHash) { Assert-Run ($state.uninstallerHash -ieq $hash) 'Uninstaller changed; refusing automatic removal.' }
    $snapshot = Product-Snapshot
    if ($state.productRegistry) { Assert-Run ($snapshot -ceq $state.productRegistry) 'Owned product registry state changed; refusing removal.' }
    $state.uninstallerHash = $hash
    $state.productRegistry = $snapshot
}
function Assert-Uninstalled {
    Assert-NoApp
    Assert-ShellUntouched
    Assert-Run (@(Registered-Products).Count -eq 0) 'PhotoForge uninstall registration remains.'
    Assert-Run (-not (Test-Path -LiteralPath $state.installDirectory)) 'Installed files remain; no directory will be force-deleted.'
    Assert-Run (-not (Test-Path -LiteralPath $machineKey)) 'Unexpected machine product state remains.'
    # Upstream silent uninstall intentionally retains the install-location key.
    # Leave this documented residue alone; do not delete/restore arbitrary registry state.
    Assert-Run ((Product-Snapshot) -ceq $state.productRegistry) 'The retained install-location registry state changed unexpectedly.'
    $state.retainedProductRegistry = $true
    $state.uninstallVerified = $true
    $state.phase = 'uninstalled-verified'
}
function Save-State {
    $state.updatedUtc = [DateTime]::UtcNow.ToString('o')
    $state | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $statePath -Encoding UTF8
}
function Assert-RunState {
    Assert-Run ((Full-Path ([IO.Path]::GetDirectoryName($RunDirectory))) -ieq $runRoot -and
        [IO.Path]::GetFileName($RunDirectory) -match '^run-[0-9a-f]{32}$') 'RunDirectory must be an immediate owned run under .work\nsis-lifecycle.'
    Assert-NoLinks $RunDirectory
    Assert-NoLinks $statePath
    Assert-Run ($state.schema -eq 1 -and $state.runDirectory -ieq $RunDirectory -and
        $state.installDirectory -ieq (Join-Path $RunDirectory 'installed') -and $state.scope -eq 'currentUser') 'Invalid or relocated run receipt.'
    $names = @($state.payloadHashes.PSObject.Properties.Name | Sort-Object)
    Assert-Run (($names -join '|') -ceq 'licenses\Tauri-MIT.txt|photoforge.exe|THIRD_PARTY_NOTICES.md') 'Receipt must pin exactly the three expected payload files.'
    foreach ($file in $state.payloadHashes.PSObject.Properties) {
        Assert-Run ($file.Value -match '^[0-9a-fA-F]{64}$') 'Invalid payload hash in receipt.'
    }
    Assert-Run (-not $state.launchVerified -and -not $state.nativeLifecycleVerified) 'GUI claims do not belong to this harness receipt.'
}
function Assert-PreviousProcessFinished {
    if ($state.processId) {
        $previous = Get-Process -Id $state.processId -ErrorAction SilentlyContinue
        Assert-Run (-not $previous) 'The previous process ID is still present (or reused); inspect it before any further action.'
        # Its exit code is unknown after a timeout, never substitute zero.
        $state.processId = $null
        $state.exitCodeUnobserved = $true
    }
}
function Invoke-OwnedProcess([string]$file, [string]$arguments, [string]$operation) {
    $expected = if ($operation -eq 'install') { $state.setup } else { Join-Path $RunDirectory 'owned-uninstaller.exe' }
    Assert-Run ($operation -in 'install', 'uninstall' -and $file -ieq $expected) 'Refusing an unowned executable/operation.'
    Assert-OrdinaryUser
    Assert-NoApp # Last check: upstream silent NSIS can kill a running same-name app.
    if ($operation -eq 'install') { Assert-WebView2Present }
    $state.phase = "$operation-requested"
    $state.lastExitCode = $null
    $state.exitCodeUnobserved = $false
    Save-State
    try { $process = Start-Process -FilePath $file -ArgumentList $arguments -WindowStyle Hidden -PassThru }
    catch { $state.phase = "$operation-not-started"; Save-State; throw }
    $state.processId = $process.Id
    Save-State
    if (-not $process.WaitForExit($TimeoutSeconds * 1000)) {
        $state.phase = "$operation-pending"
        Save-State
        throw "NSIS $operation is still running. Do not repeat it or kill it; inspect later with Verify: $RunDirectory"
    }
    $state.lastExitCode = $process.ExitCode
    $state.processId = $null
    $state.phase = "$operation-finished"
    Save-State
    Assert-Run ($state.lastExitCode -eq 0) "NSIS $operation failed with exit code $($state.lastExitCode); receipt retained."
}
function Copy-OwnedUninstaller {
    $source = Join-Path $state.installDirectory 'uninstall.exe'
    $destination = Join-Path $RunDirectory 'owned-uninstaller.exe'
    Assert-NoLinks $destination
    Assert-Run (-not (Test-Path -LiteralPath $destination)) 'An uninstaller copy already exists; do not repeat an uncertain removal.'
    Copy-Item -LiteralPath $source -Destination $destination
    Assert-Run ((Get-FileHash -LiteralPath $destination -Algorithm SHA256).Hash -ieq $state.uninstallerHash) 'Copied uninstaller hash mismatch.'
    Assert-Run ((Read-ExecutionLevel $destination) -eq 'asInvoker') 'Unexpected elevated uninstaller.'
    $destination
}

Assert-OrdinaryUser
if ($Action -in 'Verify', 'Uninstall') {
    Assert-Run ([bool]$RunDirectory -and -not $ReferenceExe -and -not $SetupSha256) 'Supply only this run receipt (and optionally the same Setup path).'
    $RunDirectory = Full-Path $RunDirectory
    $statePath = Join-Path $RunDirectory 'state.json'
    # Validate location/links BEFORE reading the receipt.
    Assert-Run ((Full-Path ([IO.Path]::GetDirectoryName($RunDirectory))) -ieq $runRoot -and
        [IO.Path]::GetFileName($RunDirectory) -match '^run-[0-9a-f]{32}$') 'RunDirectory is not an owned run.'
    Assert-NoLinks $statePath
    $state = Get-Content -LiteralPath $statePath -Raw | ConvertFrom-Json
    Assert-RunState
    if ($Setup) { Assert-Run ((Full-Path $Setup) -ieq $state.setup) 'Setup differs from this run.' }
    Assert-Package $state.setup $state.setupHash $state.version
    Assert-PreviousProcessFinished
} else {
    Assert-Run ([bool]$Setup -and [bool]$ReferenceExe -and -not $RunDirectory) 'Supply explicit Setup, SetupSha256 and exact NSIS ReferenceExe paths; no first package is selected.'
    $Setup = Full-Path $Setup
    $ReferenceExe = Full-Path $ReferenceExe
    $config = Get-Content -LiteralPath (Join-Path $repoPath 'src-tauri\tauri.conf.json') -Raw | ConvertFrom-Json
    Assert-Run ($config.bundle.windows.nsis.installMode -eq 'currentUser') 'The configured NSIS scope must remain currentUser.'
    Assert-Package $Setup $SetupSha256 $config.version
    Assert-NoLinks $ReferenceExe
    Assert-Run (Test-Path -LiteralPath $ReferenceExe -PathType Leaf) 'Exact independently retained NSIS payload reference is missing (an MSI-marked binary is not interchangeable).'
    Assert-Run ((Read-BundleTag $ReferenceExe) -ceq 'NSS') 'Reference must be the exact NSS-marked NSIS payload, not an MSI or unbundled executable.'
    $payloadHashes = [pscustomobject]@{
        'photoforge.exe'=(Get-FileHash -LiteralPath $ReferenceExe -Algorithm SHA256).Hash;
        'THIRD_PARTY_NOTICES.md'=(Get-FileHash -LiteralPath (Join-Path $repoPath 'THIRD_PARTY_NOTICES.md') -Algorithm SHA256).Hash;
        'licenses\Tauri-MIT.txt'=(Get-FileHash -LiteralPath (Join-Path $repoPath 'src-tauri\installer\Tauri-MIT.txt') -Algorithm SHA256).Hash
    }
    Assert-Pristine
    Assert-WebView2Present
    Assert-NoLinks $runRoot
    if ($Action -eq 'Preflight') {
        [pscustomobject]@{ status='preflight-pass'; setup=$Setup; setupHash=$SetupSha256.ToLowerInvariant();
            version=$config.version; scope='currentUser'; writesPerformed=$false; launchVerified=$false; nativeLifecycleVerified=$false } | ConvertTo-Json
        return
    }
    $RunDirectory = Join-Path $runRoot ('run-' + [Guid]::NewGuid().ToString('N'))
    $installDirectory = Join-Path $RunDirectory 'installed'
    Assert-Run ($installDirectory.Length -lt 240) 'Owned install path is too long for the pinned NSIS script.'
    Assert-Run (-not (Test-Path -LiteralPath $RunDirectory)) 'Run path already exists.'
    $null = New-Item -ItemType Directory -Path $RunDirectory
    $statePath = Join-Path $RunDirectory 'state.json'
    $state = [pscustomobject][ordered]@{
        schema=1; scope='currentUser'; runDirectory=$RunDirectory; installDirectory=$installDirectory;
        setup=$Setup; setupHash=$SetupSha256.ToLowerInvariant(); version=$config.version;
        payloadHashes=$payloadHashes;
        productRegistry=$null; uninstallerHash=$null; phase='preflight-pass';
        createdUtc=[DateTime]::UtcNow.ToString('o'); updatedUtc=$null; processId=$null;
        lastExitCode=$null; exitCodeUnobserved=$false; lastError=$null; installedVerified=$false;
        uninstallVerified=$false; retainedProductRegistry=$false; launchVerified=$false; nativeLifecycleVerified=$false
    }
    Save-State
    Write-Output "RunDirectory: $RunDirectory"
}
try {
    switch ($Action) {
        'Install' {
            Assert-Pristine
            # /D must be LAST and unquoted even for a path containing spaces (NSIS manual).
            # /NS suppresses shortcuts, and NO /R means NO automatic application launch.
            Invoke-OwnedProcess $state.setup ('/S /NS /D=' + $state.installDirectory) 'install'
            Assert-Installed
            $state.installedVerified = $true
            $state.phase = 'installed-verified'
        }
        'Verify' {
            Assert-Run ($state.phase -notin 'preflight-pass', 'install-not-started', 'uninstalled-verified') 'This receipt has no pending/current installation to verify.'
            if ($state.phase -like 'uninstall-*') { Assert-Uninstalled }
            else { Assert-Installed; $state.installedVerified = $true; $state.phase = 'installed-verified' }
        }
        'Uninstall' {
            Assert-Run ($state.installedVerified -and $state.phase -in 'installed-verified', 'uninstall-not-started') 'Removal requires a verified owned installation; pending/finished operations are never repeated.'
            Assert-Installed
            $copy = Copy-OwnedUninstaller
            # Run an owned COPY outside INSTDIR so the original can be removed. _?=
            # disables the temp relaunch: WaitForExit then observes the actual uninstaller.
            # /UPDATE preserves personal jump lists/autostart/shortcuts. This deliberately
            # tests payload/registration removal, NOT the full normal shortcut lifecycle.
            Invoke-OwnedProcess $copy ('/S /UPDATE _?=' + $state.installDirectory) 'uninstall'
            Assert-Uninstalled
        }
    }
    Save-State
    $state | ConvertTo-Json -Depth 8
} catch {
    $state.lastError = $_.Exception.Message
    Save-State
    throw
}
