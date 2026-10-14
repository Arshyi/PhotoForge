# Hermetic guard tests. Loads only function definitions from the actual harness;
# every registry/process/filesystem/elevation boundary below is a mock. No MSI,
# UAC prompt, installed product, file write or deletion is used by these tests.
$ErrorActionPreference = 'Stop'
# Import before shadowing Get-FileHash: its module also supplies Add-Member, which
# would otherwise lazily import and replace that function during the first mock.
Import-Module Microsoft.PowerShell.Utility
$tokens = $null
$parseErrors = $null
$ast = [Management.Automation.Language.Parser]::ParseFile(
    (Join-Path $PSScriptRoot 'msi_lifecycle.ps1'), [ref]$tokens, [ref]$parseErrors)
if ($parseErrors.Count) { throw ($parseErrors | Out-String) }
foreach ($definition in $ast.FindAll({ param($node) $node -is [Management.Automation.Language.FunctionDefinitionAst] }, $false)) {
    . ([scriptblock]::Create($definition.Extent.Text))
}
$machinePath = 'C:\Program Files\PhotoForge'
$machineExe = Join-Path $machinePath 'photoforge.exe'
$machineKey = 'HKLM:\Software\photoforge\PhotoForge'
$userKey = 'HKCU:\Software\photoforge\PhotoForge'
$commonDesktop = [Environment]::GetFolderPath('CommonDesktopDirectory')
$RunDirectory = 'C:\mock\run'
$ElevationTimeoutSeconds = 1
$script:passed = 0
function Reset-Mocks {
    $script:products = @()
    $script:running = @()
    $script:paths = @{ $commonDesktop=$true }
    $script:linked = @{}
    $script:userSnapshot = 'absent'
    $script:registeredPath = $machinePath
    $script:fakeHash = 'A' * 64
    $script:elevation = 'success'
    $script:exitCode = 0
    $script:launchCalls = 0
    $script:phases = @()
    $script:staleDirectory = 'C:\mock\stale-nsis'
    $script:state = [pscustomobject]@{ productCode='{11111111-2222-3333-4444-555555555555}';
        version='0.15.2'; userRegistryBefore='absent'; installedExeHash=$null;
        phase='preflight-pass'; msiexecPid=$null; lastExitCode=$null; lastError=$null; restartRequired=$false; uninstallVerified=$false }
}
function Registered-Products { $script:products }
function Get-Process { param($Name, $Id, $ErrorAction) $script:running }
function Test-Path { param($LiteralPath, $PathType) $script:paths.ContainsKey($LiteralPath) -and $script:paths[$LiteralPath] }
function Get-Item {
    param($LiteralPath, [switch]$Force)
    $item = [pscustomobject]@{ Attributes=$(if ($script:linked[$LiteralPath]) { [IO.FileAttributes]::ReparsePoint } else { [IO.FileAttributes]::Directory }) }
    $item | Add-Member ScriptMethod GetValue { param($Name) $script:staleDirectory }
    $item | Add-Member ScriptMethod Close { }
    $item
}
function Get-ItemProperty { param($LiteralPath, $Name) [pscustomobject]@{ InstallDir=$script:registeredPath } }
function Get-FileHash { param($LiteralPath, $Algorithm) [pscustomobject]@{ Hash=$script:fakeHash } }
function User-RegistrySnapshot { $script:userSnapshot }
function Save-State { $script:phases += $state.phase }
function Start-Process {
    param($FilePath, $ArgumentList, $Verb, $WindowStyle, [switch]$PassThru)
    $script:launchCalls++
    if ($Verb -ne 'RunAs' -or $WindowStyle -ne 'Hidden') { throw 'Unexpected elevation boundary.' }
    if ($script:elevation -eq 'cancel') { throw (New-Object ComponentModel.Win32Exception 1223) }
    $process = [pscustomobject]@{ Id=12345; ExitCode=$script:exitCode }
    $process | Add-Member ScriptMethod WaitForExit { param($Timeout) $script:elevation -ne 'timeout' }
    $process
}
function Expect-Refusal([scriptblock]$Body, [string]$Pattern) {
    try { & $Body } catch {
        if ($_.Exception.Message -notmatch $Pattern) { throw "Unexpected refusal: $($_.Exception.Message)" }
        return
    }
    throw "Expected refusal matching $Pattern"
}
function Test-Guard([string]$Name, [scriptblock]$Body) {
    Reset-Mocks
    & $Body
    $script:passed++
    Write-Output "PASS $Name"
}
function Set-Installed {
    $script:products = @([pscustomobject]@{ key=$state.productCode;
        hive='HKLM:\Software\Microsoft\Windows\CurrentVersion\Uninstall'; version=$state.version })
    foreach ($path in $machineExe, (Join-Path $machinePath 'THIRD_PARTY_NOTICES.md'),
        (Join-Path $machinePath 'licenses\Tauri-MIT.txt')) { $script:paths[$path] = $true }
}

Test-Guard 'pristine preflight preserves a Desktop project folder and invokes no elevation' {
    $script:paths[(Join-Path ([Environment]::GetFolderPath('Desktop')) 'PhotoForge')] = $true
    Assert-Pristine
    if ($script:launchCalls -ne 0) { throw 'Preflight elevated.' }
}
Test-Guard 'existing registered product is protected' {
    $script:products = @([pscustomobject]@{ key='existing' })
    Expect-Refusal { Assert-Pristine } 'already registered'
}
Test-Guard 'running app is never closed' {
    $script:running = @(42)
    Expect-Refusal { Assert-Pristine } 'Close PhotoForge yourself'
}
Test-Guard 'existing machine directory is protected' {
    $script:paths[$machinePath] = $true
    Expect-Refusal { Assert-Pristine } 'Existing directory'
}
Test-Guard 'existing machine registry state is protected' {
    $script:paths[$machineKey] = $true
    Expect-Refusal { Assert-Pristine } 'Existing machine product state'
}
Test-Guard 'unregistered per-user executable is protected' {
    $script:paths[(Join-Path $env:LOCALAPPDATA 'PhotoForge\photoforge.exe')] = $true
    Expect-Refusal { Assert-Pristine } 'unregistered per-user'
}
Test-Guard 'existing shared shortcut is protected' {
    $script:paths[(Join-Path $commonDesktop 'PhotoForge.lnk')] = $true
    Expect-Refusal { Assert-Pristine } 'Existing PhotoForge shortcut'
}
Test-Guard 'stale NSIS path with an executable is protected' {
    $script:paths[$userKey] = $true
    $script:paths[(Join-Path $script:staleDirectory 'photoforge.exe')] = $true
    Expect-Refusal { Assert-Pristine } 'retained per-user path'
}
Test-Guard 'linked path refuses inspection' {
    $script:paths[$machinePath] = $true
    $script:linked[$machinePath] = $true
    Expect-Refusal { Assert-NoLinks $machinePath } 'linked path'
}
Test-Guard 'missing shared Desktop refuses preflight' {
    $script:paths.Remove($commonDesktop)
    Expect-Refusal { Assert-Pristine } 'shared Desktop'
}
Test-Guard 'owned machine registration and resources can be verified' {
    Set-Installed
    Assert-Installed
    if ($state.installedExeHash -ne $script:fakeHash) { throw 'Payload hash was not retained.' }
}
Test-Guard 'uninstall cannot use an arbitrary registered product' {
    Set-Installed
    $script:products[0].key = '{99999999-2222-3333-4444-555555555555}'
    Expect-Refusal { Assert-Installed } 'exact machine product'
}
Test-Guard 'wrong scope or version does not establish ownership' {
    Set-Installed
    $script:products[0].hive = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall'
    Expect-Refusal { Assert-Installed } 'exact machine product'
    Set-Installed
    $script:products[0].version = '0.15.1'
    Expect-Refusal { Assert-Installed } 'exact machine product'
}
Test-Guard 'redirected install directory is refused' {
    Set-Installed
    $script:registeredPath = 'C:\Users\mock\PhotoForge'
    Expect-Refusal { Assert-Installed } 'expected default'
}
Test-Guard 'changed per-user registry state is not ignored' {
    Set-Installed
    $script:userSnapshot = 'changed'
    Expect-Refusal { Assert-Installed } 'Per-user NSIS product state changed'
}
Test-Guard 'missing installed notice is a failure' {
    Set-Installed
    $script:paths.Remove((Join-Path $machinePath 'licenses\Tauri-MIT.txt'))
    Expect-Refusal { Assert-Installed } 'resource is missing'
}
Test-Guard 'changed executable blocks automatic removal' {
    Set-Installed
    $state.installedExeHash = 'B' * 64
    Expect-Refusal { Assert-Installed } 'Executable changed'
}
Test-Guard 'UAC cancellation is recorded without success' {
    $script:elevation = 'cancel'
    Expect-Refusal { Elevate-Msi 'mock arguments' 'install' } 'cancel'
    if ($state.phase -ne 'install-not-started' -or $null -ne $state.lastExitCode) { throw 'Cancellation was misreported.' }
}
Test-Guard 'timeout preserves pending process and does not claim a pass' {
    $script:elevation = 'timeout'
    Expect-Refusal { Elevate-Msi 'mock arguments' 'install' } 'still running'
    if ($state.phase -ne 'install-pending' -or $state.msiexecPid -ne 12345) { throw 'Pending process was lost.' }
}
Test-Guard 'nonzero MSI exit is not a pass' {
    $script:exitCode = 1603
    Expect-Refusal { Elevate-Msi 'mock arguments' 'install' } '1603'
    if ($state.lastExitCode -ne 1603 -or $null -ne $state.msiexecPid) { throw 'Failure exit was lost.' }
}
Test-Guard 'zero MSI exit is successful but not launch/GUI proof' {
    $installStrings = @($ast.FindAll({ param($node) $node -is [Management.Automation.Language.StringConstantExpressionAst] -and $node.Value -like '/i * /qn *' }, $true))
    if ($installStrings.Count -ne 1 -or $installStrings[0].Value -match 'AUTOLAUNCHAPP=') { throw 'Silent installation must not set a truthy auto-launch property.' }
    Elevate-Msi 'mock arguments' 'install'
    if ($state.lastExitCode -ne 0 -or $state.restartRequired) { throw 'Incorrect zero-exit handling.' }
}
Test-Guard 'restart-required success is explicitly retained' {
    $script:exitCode = 3010
    Elevate-Msi 'mock arguments' 'uninstall'
    if (-not $state.restartRequired) { throw 'Restart requirement was discarded.' }
}
Test-Guard 'removal verifies absence without deleting anything' {
    Assert-Uninstalled
    if (-not $state.uninstallVerified -or $state.phase -ne 'uninstalled-verified') { throw 'Removal was not recorded.' }
}
Test-Guard 'leftover files or registration fail removal verification' {
    $script:paths[$machinePath] = $true
    Expect-Refusal { Assert-Uninstalled } 'Files remain'
    $script:paths.Remove($machinePath)
    $script:products = @(1)
    Expect-Refusal { Assert-Uninstalled } 'registration remains'
}
Test-Guard 'shared Desktop and per-user state must survive removal' {
    $script:paths.Remove($commonDesktop)
    Expect-Refusal { Assert-Uninstalled } 'shared Desktop'
    $script:paths[$commonDesktop] = $true
    $script:userSnapshot = 'changed'
    Expect-Refusal { Assert-Uninstalled } 'per-user NSIS state'
}
Test-Guard 'leftover machine shortcuts fail removal verification' {
    $script:paths[(Join-Path $commonDesktop 'PhotoForge.lnk')] = $true
    Expect-Refusal { Assert-Uninstalled } 'Desktop shortcut remains'
}
[pscustomobject]@{ tests=$script:passed; status='pass'; mockedBoundaries=$true; nativeLifecycleVerified=$false } | ConvertTo-Json
