# Hermetic tests: load actual function/entry ASTs, mock every process/registry/file
# boundary. No installer, UAC, app window, clipboard or native PE API is invoked.
$ErrorActionPreference = 'Stop'
Import-Module Microsoft.PowerShell.Utility
$tokens = $null
$parseErrors = $null
$ast = [Management.Automation.Language.Parser]::ParseFile(
    (Join-Path $PSScriptRoot 'nsis_lifecycle.ps1'), [ref]$tokens, [ref]$parseErrors)
if ($parseErrors.Count) { throw ($parseErrors | Out-String) }
foreach ($definition in $ast.FindAll({ param($node) $node -is [Management.Automation.Language.FunctionDefinitionAst] }, $false)) {
    . ([scriptblock]::Create($definition.Extent.Text))
}
$entryText = ($ast.EndBlock.Statements | Where-Object {
    $_ -isnot [Management.Automation.Language.FunctionDefinitionAst]
} | ForEach-Object { $_.Extent.Text }) -join [Environment]::NewLine
# Dynamically compiled blocks have no PSScriptRoot; supply a mock source location.
$entry = [scriptblock]::Create($entryText.Replace('$PSScriptRoot', "'C:\mock\tools\validate'"))
$runRoot = 'C:\mock\.work\nsis-lifecycle'
$RunDirectory = Join-Path $runRoot ('run-' + ('1' * 32))
$statePath = Join-Path $RunDirectory 'state.json'
$userKey = 'HKCU:\Software\photoforge\PhotoForge'
$machineKey = 'HKLM:\Software\photoforge\PhotoForge'
$userUninstallHive = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall'
$runKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run'
$TimeoutSeconds = 1
$script:passed = 0

function Reset-Mocks {
    $script:paths = @{ 'C:\mock\setup.exe'=$true; 'C:\mock\reference.exe'=$true }
    $script:paths['HKLM:\Software\WOW6432Node\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}']=$true
    $script:runtimeVersion='147.0.0.0'
    $script:linked = @{}
    $script:unreadable = @{}
    $script:missing = @{}
    $script:hashes = @{}
    $script:products = @()
    $script:running = @()
    $script:pending = @()
    $script:admin = $false
    $script:manifestLevel = 'asInvoker'
    $script:bundleTag = 'NSS'
    $script:version = '0.15.2'
    $script:productName = 'PhotoForge'
    $script:registryValues = @{ ''=(Join-Path $RunDirectory 'installed') }
    $script:subkeys = @()
    $script:launches = @()
    $script:copies = @()
    $script:phases = @()
    $script:newDirectories = @()
    $script:exitCode = 0
    $script:processOutcome = 'success'
    $script:onLaunch = $null
    $script:state = [pscustomobject]@{
        schema=1; scope='currentUser'; runDirectory=$RunDirectory; installDirectory=(Join-Path $RunDirectory 'installed');
        setup='C:\mock\setup.exe'; setupHash=('A' * 64); version='0.15.2';
        payloadHashes=[pscustomobject]@{ 'photoforge.exe'=('A' * 64); 'THIRD_PARTY_NOTICES.md'=('A' * 64); 'licenses\Tauri-MIT.txt'=('A' * 64) };
        productRegistry=$null; uninstallerHash=$null; phase='preflight-pass'; processId=$null;
        lastExitCode=$null; lastError=$null; exitCodeUnobserved=$false; installedVerified=$false; uninstallVerified=$false;
        retainedProductRegistry=$false; launchVerified=$false; nativeLifecycleVerified=$false
    }
    $script:rootItems = @(
        [pscustomobject]@{ Name='licenses'; PSIsContainer=$true; FullName=(Join-Path $state.installDirectory 'licenses') }
        foreach ($name in 'photoforge.exe', 'uninstall.exe', 'THIRD_PARTY_NOTICES.md') {
            [pscustomobject]@{ Name=$name; PSIsContainer=$false; FullName=(Join-Path $state.installDirectory $name) }
        }
    )
    $script:licenseItems = @([pscustomobject]@{ Name='Tauri-MIT.txt'; PSIsContainer=$false;
        FullName=(Join-Path $state.installDirectory 'licenses\Tauri-MIT.txt') })
}
function Assert-OrdinaryUser { if ($script:admin) { throw 'Use an ordinary session.' } }
function Registered-Products { $script:products }
function Get-Process {
    param($Name, $Id, $ErrorAction)
    if ($Id) { $script:pending } else { $script:running }
}
function Test-Path { param($LiteralPath, $PathType) $script:paths.ContainsKey($LiteralPath) -and $script:paths[$LiteralPath] }
function Get-Item {
    param($LiteralPath, [switch]$Force, $ErrorAction)
    if ($script:unreadable[$LiteralPath]) { throw [UnauthorizedAccessException]::new('Mock path access denied.') }
    if ($script:missing[$LiteralPath]) { throw [Management.Automation.ItemNotFoundException]::new('Mock missing entry.') }
    $item = [pscustomobject]@{
        Attributes=$(if ($script:linked[$LiteralPath]) { [IO.FileAttributes]::ReparsePoint } else { [IO.FileAttributes]::Directory });
        VersionInfo=[pscustomobject]@{ ProductName=$script:productName; ProductVersion=$script:version; FileVersion=$script:version }
    }
    $item | Add-Member ScriptMethod GetValueNames { @($script:registryValues.Keys) }
    $item | Add-Member ScriptMethod GetValue { param($Name) $script:registryValues[$Name] }
    $item | Add-Member ScriptMethod GetValueKind { param($Name) 'String' }
    $item | Add-Member ScriptMethod GetSubKeyNames { $script:subkeys }
    $item | Add-Member ScriptMethod Close {}
    $item
}
function Get-ChildItem {
    param($LiteralPath, [switch]$Force)
    if ($LiteralPath -eq $state.installDirectory) { $script:rootItems }
    elseif ($LiteralPath -eq (Join-Path $state.installDirectory 'licenses')) { $script:licenseItems }
    else { throw "Unexpected directory enumeration: $LiteralPath" }
}
function Get-ItemProperty {
    param($LiteralPath, $Name, $ErrorAction)
    if ($LiteralPath -notlike '*EdgeUpdate\Clients\*' -or $Name -ne 'pv') { throw 'Unexpected registry value read.' }
    [pscustomobject]@{ pv=$script:runtimeVersion }
}
function Get-FileHash {
    param($LiteralPath, $Algorithm)
    [pscustomobject]@{ Hash=$(if ($script:hashes.ContainsKey($LiteralPath)) { $script:hashes[$LiteralPath] } else { 'A' * 64 }) }
}
function Read-ExecutionLevel { param($path) $script:manifestLevel }
function Read-BundleTag { param($path) $script:bundleTag }
function Save-State { $script:phases += $state.phase }
function Copy-Item {
    param($LiteralPath, $Destination)
    if ($LiteralPath -ne (Join-Path $state.installDirectory 'uninstall.exe') -or $Destination -ne (Join-Path $RunDirectory 'owned-uninstaller.exe')) { throw 'Unexpected copy target.' }
    $script:copies += $Destination
    $script:paths[$Destination] = $true
}
function Start-Process {
    param($FilePath, $ArgumentList, $WindowStyle, $Verb, [switch]$PassThru)
    if ($Verb -or $WindowStyle -ne 'Hidden') { throw 'Unexpected visible/elevated execution.' }
    $script:launches += [pscustomobject]@{ file=$FilePath; arguments=$ArgumentList }
    if ($script:processOutcome -eq 'start-failed') { throw 'Mock process start refused.' }
    if ($script:onLaunch) { & $script:onLaunch }
    $process = [pscustomobject]@{ Id=12345; ExitCode=$script:exitCode }
    $process | Add-Member ScriptMethod WaitForExit { param($Timeout) $script:processOutcome -ne 'timeout' }
    $process
}
function New-Item {
    param($ItemType, $Path)
    $script:newDirectories += $Path
    $script:paths[$Path] = $true
}
function Get-Content {
    param($LiteralPath, [switch]$Raw)
    if ($LiteralPath -like '*tauri.conf.json') { '{"version":"0.15.2","bundle":{"windows":{"nsis":{"installMode":"currentUser"}}}}' }
    elseif ($LiteralPath -like '*state.json') { $script:receiptJson }
    else { throw "Unexpected file read: $LiteralPath" }
}
function Resolve-Path { param($LiteralPath) [pscustomobject]@{ ProviderPath='C:\mock' } }
function Expect-Refusal([scriptblock]$Body, [string]$Pattern) {
    try { & $Body | Out-Null } catch {
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
    $script:registryValues[''] = $state.installDirectory
    $script:rootItems = @(
        [pscustomobject]@{ Name='licenses'; PSIsContainer=$true; FullName=(Join-Path $state.installDirectory 'licenses') }
        foreach ($name in 'photoforge.exe', 'uninstall.exe', 'THIRD_PARTY_NOTICES.md') {
            [pscustomobject]@{ Name=$name; PSIsContainer=$false; FullName=(Join-Path $state.installDirectory $name) }
        }
    )
    $script:licenseItems = @([pscustomobject]@{ Name='Tauri-MIT.txt'; PSIsContainer=$false;
        FullName=(Join-Path $state.installDirectory 'licenses\Tauri-MIT.txt') })
    $script:products = @([pscustomobject]@{
        key='PhotoForge'; hive=$userUninstallHive; version=$state.version; publisher='photoforge';
        location=('"' + $state.installDirectory + '"'); uninstall=('"' + (Join-Path $state.installDirectory 'uninstall.exe') + '"');
        mainBinary='photoforge.exe'
    })
    $script:paths[$userKey] = $true
    $script:paths[$state.installDirectory] = $true
    foreach ($item in @($script:rootItems) + @($script:licenseItems)) { $script:paths[$item.FullName] = $true }
}

Test-Guard 'entry defaults to read-only Preflight' {
    $actionParameter = @($ast.ParamBlock.Parameters | Where-Object { $_.Name.VariablePath.UserPath -eq 'Action' })[0]
    if ($actionParameter.DefaultValue.Value -ne 'Preflight') { throw 'Default must be read-only.' }
    $Action = 'Preflight'; $Setup='C:\mock\setup.exe'; $SetupSha256='A' * 64; $ReferenceExe='C:\mock\reference.exe'; $RunDirectory=''
    $result = & $entry | ConvertFrom-Json
    if ($result.status -ne 'preflight-pass' -or $result.writesPerformed -or $script:launches.Count -or $script:newDirectories.Count -or $script:copies.Count -or $script:phases.Count) { throw 'Preflight wrote or launched.' }
}
Test-Guard 'entry refuses missing explicit inputs without writes' {
    $Action='Preflight'; $Setup=''; $SetupSha256=''; $ReferenceExe=''; $RunDirectory=''
    Expect-Refusal { & $entry } 'explicit Setup'
    if ($script:launches.Count -or $script:newDirectories.Count) { throw 'Refusal had side effects.' }
}
Test-Guard 'explicit Install entry uses a fresh owned path and no app/shortcut launch' {
    $Action='Install'; $Setup='C:\mock\setup.exe'; $SetupSha256='A' * 64; $ReferenceExe='C:\mock\reference.exe'; $RunDirectory=''
    $script:onLaunch={ Set-Installed }
    $output=@(& $entry)
    $result=$output[-1] | ConvertFrom-Json
    if ($result.phase -ne 'installed-verified' -or -not $result.installedVerified -or $result.launchVerified -or
        $script:newDirectories.Count -ne 1 -or $script:launches.Count -ne 1 -or
        $script:launches[0].arguments -cne ('/S /NS /D=' + $result.installDirectory)) { throw 'Install entry policy failed.' }
}
Test-Guard 'explicit Uninstall entry executes only the verified copy and preserves shell state' {
    Set-Installed; Assert-Installed
    $state.installedVerified=$true; $state.phase='installed-verified'
    $script:receiptJson=$state | ConvertTo-Json -Depth 8
    $script:onLaunch={ $script:products=@(); $script:paths.Remove($state.installDirectory) }
    $Action='Uninstall'; $Setup=''; $SetupSha256=''; $ReferenceExe=''
    $result=& $entry | ConvertFrom-Json
    if ($result.phase -ne 'uninstalled-verified' -or -not $result.uninstallVerified -or $script:launches.Count -ne 1 -or
        $script:copies.Count -ne 1 -or $script:launches[0].file -ne (Join-Path $RunDirectory 'owned-uninstaller.exe') -or
        $script:launches[0].arguments -cne ('/S /UPDATE _?=' + $state.installDirectory)) { throw 'Uninstall entry policy failed.' }
}
Test-Guard 'pending Uninstall entry cannot be repeated even after its PID disappears' {
    Set-Installed; Assert-Installed
    $state.installedVerified=$true; $state.phase='uninstall-pending'; $state.processId=12345
    $script:receiptJson=$state | ConvertTo-Json -Depth 8
    $Action='Uninstall'; $Setup=''; $SetupSha256=''; $ReferenceExe=''
    Expect-Refusal { & $entry } 'never repeated'
    if ($script:launches.Count -or $script:copies.Count) { throw 'Pending removal was repeated.' }
}
Test-Guard 'local paths only and no command-line quotes/control characters' {
    foreach ($path in '\\server\share\setup.exe', 'relative.exe', ('C:\mock\bad' + [char]10 + 'path'), 'C:\mock\"bad') {
        Expect-Refusal { Full-Path $path } 'absolute local'
    }
}
Test-Guard 'existing Desktop project folder is not an install target' {
    $script:paths[(Join-Path ([Environment]::GetFolderPath('Desktop')) 'PhotoForge')] = $true
    Assert-Pristine
}
Test-Guard 'existing registered products are protected' {
    $script:products = @(1)
    Expect-Refusal { Assert-Pristine } 'already registered'
}
Test-Guard 'stale per-user registry is protected instead of overwritten' {
    $script:paths[$userKey] = $true
    Expect-Refusal { Assert-Pristine } 'even stale'
}
Test-Guard 'machine registry and default directories are protected' {
    $script:paths[$machineKey] = $true
    Expect-Refusal { Assert-Pristine } 'registry state'
    $script:paths.Remove($machineKey)
    $script:paths[(Join-Path $env:LOCALAPPDATA 'PhotoForge')] = $true
    Expect-Refusal { Assert-Pristine } 'directory will not be replaced'
}
Test-Guard 'running app is never closed or killed' {
    $script:running = @(42)
    Expect-Refusal { Assert-Pristine } 'Close PhotoForge yourself'
    if ($script:launches.Count) { throw 'Installer launched while app was running.' }
}
Test-Guard 'shortcuts and autostart state are protected' {
    $shortcut = @(Shortcut-Paths)[0]
    $script:paths[$shortcut] = $true
    Expect-Refusal { Assert-ShellUntouched } 'shortcut state'
    $script:paths.Remove($shortcut)
    $script:paths[$runKey] = $true; $script:registryValues=@{ PhotoForge='retained' }
    Expect-Refusal { Assert-ShellUntouched } 'autostart entry'
}
Test-Guard 'linked ancestors are rejected' {
    $script:paths['C:\mock'] = $true; $script:linked['C:\mock'] = $true
    Expect-Refusal { Assert-NoLinks 'C:\mock\child\setup.exe' } 'linked path'
}
Test-Guard 'dangling links cannot hide behind a false Test-Path result' {
    $script:linked['C:\mock\dangling'] = $true
    Expect-Refusal { Assert-NoLinks 'C:\mock\dangling\child' } 'linked path'
}
Test-Guard 'inaccessible path inspection fails closed' {
    $script:unreadable['C:\mock\private'] = $true
    Expect-Refusal { Assert-NoLinks 'C:\mock\private\child' } 'access denied'
}
Test-Guard 'genuinely missing entries may be inspected without being created' {
    $script:missing['C:\mock\new\child'] = $true
    $script:missing['C:\mock\new'] = $true
    Assert-NoLinks 'C:\mock\new\child'
    if ($script:newDirectories.Count) { throw 'Missing-entry inspection created a directory.' }
}
Test-Guard 'installer hash and version are pinned' {
    Assert-Package $state.setup $state.setupHash $state.version
    Expect-Refusal { Assert-Package $state.setup 'not-a-hash' $state.version } 'SetupSha256'
    Expect-Refusal { Assert-Package $state.setup ('B' * 64) $state.version } 'hash changed'
    $script:version='0.15.1'
    Expect-Refusal { Assert-Package $state.setup $state.setupHash $state.version } 'identity/version'
}
Test-Guard 'elevated manifest is rejected before process launch' {
    $script:manifestLevel='requireAdministrator'
    Expect-Refusal { Assert-Package $state.setup $state.setupHash $state.version } 'asInvoker'
}
Test-Guard 'missing WebView2 refuses provisioning or installer launch' {
    $script:paths.Remove('HKLM:\Software\WOW6432Node\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}')
    Expect-Refusal { Invoke-OwnedProcess $state.setup '/S' 'install' } 'will not download'
    if ($script:launches.Count) { throw 'Missing prerequisite started provisioning.' }
}
Test-Guard 'per-user WebView2 fallback is allowed but invalid versions are refused' {
    $script:paths.Remove('HKLM:\Software\WOW6432Node\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}')
    $script:paths['HKCU:\Software\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}']=$true
    Assert-WebView2Present
    $script:runtimeVersion='invalid'
    Expect-Refusal { Assert-WebView2Present } 'missing/invalid'
}
Test-Guard 'streaming bundle marker survives a read-buffer boundary' {
    $bytes=New-Object byte[] 65600
    $marker=[Text.Encoding]::ASCII.GetBytes('__TAURI_BUNDLE_TYPE_VAR_NSS')
    [Buffer]::BlockCopy($marker, 0, $bytes, 65530, $marker.Length)
    $stream=New-Object IO.MemoryStream (,$bytes)
    try { if ((Bundle-TagFromStream $stream) -cne 'NSS') { throw 'Split bundle marker was lost.' } }
    finally { $stream.Dispose() }
}
Test-Guard 'missing and duplicate bundle markers cannot qualify a reference' {
    foreach ($text in 'not-a-bundle', '__TAURI_BUNDLE_TYPE_VAR_NSS__TAURI_BUNDLE_TYPE_VAR_MSI') {
        $stream=New-Object IO.MemoryStream (,[Text.Encoding]::ASCII.GetBytes($text))
        try { Expect-Refusal { Bundle-TagFromStream $stream } 'ambiguous|unique' }
        finally { $stream.Dispose() }
    }
}
Test-Guard 'entry refuses an MSI-marked payload reference without writes' {
    $Action='Preflight'; $Setup='C:\mock\setup.exe'; $SetupSha256='A' * 64; $ReferenceExe='C:\mock\reference.exe'; $RunDirectory=''
    $script:bundleTag='MSI'
    Expect-Refusal { & $entry } 'NSS-marked'
    if ($script:launches.Count -or $script:newDirectories.Count) { throw 'Wrong-bundle refusal had side effects.' }
}
Test-Guard 'owned layout and exact payload hashes are verified' {
    Set-Installed
    Assert-Installed
    if ($state.uninstallerHash -ne ('A' * 64) -or -not $state.productRegistry) { throw 'Ownership hashes were not retained.' }
}
Test-Guard 'wrong registry scope or version cannot establish ownership' {
    Set-Installed
    $script:products[0].hive='HKLM:\Software\Microsoft\Windows\CurrentVersion\Uninstall'
    Expect-Refusal { Assert-Installed } 'exact per-user'
    Set-Installed; $script:products[0].version='0.15.1'
    Expect-Refusal { Assert-Installed } 'exact per-user'
}
Test-Guard 'redirected registry uninstall command is never executed' {
    Set-Installed; $script:products[0].uninstall='"C:\other\uninstall.exe"'
    Expect-Refusal { Assert-Installed } 'exact per-user'
    if ($script:launches.Count) { throw 'Registry command executed.' }
}
Test-Guard 'extra installed files protect user additions from uninstall' {
    Set-Installed
    $script:rootItems += [pscustomobject]@{ Name='personal.photoforge'; PSIsContainer=$false; FullName=(Join-Path $state.installDirectory 'personal.photoforge') }
    Expect-Refusal { Assert-Installed } 'Unexpected installed files'
}
Test-Guard 'unexpected subdirectories and license contents are protected' {
    Set-Installed; $script:rootItems[0].Name='personal'
    Expect-Refusal { Assert-Installed } 'Unexpected installed entry'
    Set-Installed; $script:rootItems[0].Name='licenses'
    $script:licenseItems[0].Name='personal.txt'
    Expect-Refusal { Assert-Installed } 'license directory contents'
}
Test-Guard 'linked installed payload cannot be removed' {
    Set-Installed
    $script:linked[(Join-Path $state.installDirectory 'photoforge.exe')] = $true
    Expect-Refusal { Assert-Installed } 'linked path'
}
Test-Guard 'changed app or notice hash blocks uninstall' {
    Set-Installed
    $script:hashes[(Join-Path $state.installDirectory 'photoforge.exe')]='B' * 64
    Expect-Refusal { Assert-Installed } 'payload changed'
    $script:hashes.Clear()
    $script:hashes[(Join-Path $state.installDirectory 'licenses\Tauri-MIT.txt')]='B' * 64
    Expect-Refusal { Assert-Installed } 'payload changed'
}
Test-Guard 'changed uninstaller hash blocks automatic removal' {
    Set-Installed; $state.uninstallerHash='B' * 64
    Expect-Refusal { Assert-Installed } 'Uninstaller changed'
}
Test-Guard 'product path and unknown registry subkeys/values are protected' {
    Set-Installed; $script:registryValues['']='C:\other'
    Expect-Refusal { Product-Snapshot } 'redirected'
    $script:registryValues['']=$state.installDirectory; $script:subkeys=@('personal')
    Expect-Refusal { Product-Snapshot } 'subkeys'
    $script:subkeys=@(); $script:registryValues['personal']='retained'
    Expect-Refusal { Product-Snapshot } 'registry value'
}
Test-Guard 'changed product snapshot blocks automatic removal' {
    Set-Installed; Assert-Installed
    $script:registryValues['Installer Language']='1033'
    Expect-Refusal { Assert-Installed } 'registry state changed'
}
Test-Guard 'receipt relocation and nested run directories are rejected' {
    Assert-RunState
    $state.installDirectory='C:\other'
    Expect-Refusal { Assert-RunState } 'relocated'
    $RunDirectory=Join-Path (Join-Path $runRoot 'nested') ('run-' + ('2' * 32))
    Expect-Refusal { Assert-RunState } 'immediate owned run'
}
Test-Guard 'receipt cannot add payload paths or GUI claims' {
    $state.payloadHashes | Add-Member NoteProperty '..\personal' ('A' * 64)
    Expect-Refusal { Assert-RunState } 'exactly the three'
    $state.payloadHashes.PSObject.Properties.Remove('..\personal')
    $state.launchVerified=$true
    Expect-Refusal { Assert-RunState } 'GUI claims'
}
Test-Guard 'process start failure is not an install pass' {
    $script:processOutcome='start-failed'
    Expect-Refusal { Invoke-OwnedProcess $state.setup '/S /NS /D=C:\mock\installed' 'install' } 'start refused'
    if ($state.phase -ne 'install-not-started' -or $state.lastExitCode) { throw 'Incorrect start failure state.' }
}
Test-Guard 'timeout preserves pending identity without killing it' {
    $script:processOutcome='timeout'
    Expect-Refusal { Invoke-OwnedProcess $state.setup '/S /NS /D=C:\mock\installed' 'install' } 'still running'
    if ($state.phase -ne 'install-pending' -or $state.processId -ne 12345 -or $null -ne $state.lastExitCode) { throw 'Pending process was misreported.' }
    $script:pending=@(12345)
    Expect-Refusal { Assert-PreviousProcessFinished } 'process ID is still present'
}
Test-Guard 'finished timeout never substitutes an unknown exit code with zero' {
    $state.processId=12345; $state.lastExitCode=$null
    Assert-PreviousProcessFinished
    if ($state.processId -or -not $state.exitCodeUnobserved -or $null -ne $state.lastExitCode) { throw 'Unknown exit code was lost.' }
}
Test-Guard 'nonzero process exit is a failure' {
    $script:exitCode=2
    Expect-Refusal { Invoke-OwnedProcess $state.setup '/S /NS /D=C:\mock\installed' 'install' } 'exit code 2'
    if ($state.lastExitCode -ne 2 -or $state.phase -ne 'install-finished') { throw 'Failure result was lost.' }
}
Test-Guard 'zero exit is retained without launch/native claims' {
    Invoke-OwnedProcess $state.setup '/S /NS /D=C:\mock\installed' 'install'
    if ($state.lastExitCode -ne 0 -or $state.launchVerified -or $state.nativeLifecycleVerified) { throw 'Incorrect process success claims.' }
}
Test-Guard 'arbitrary process paths and elevated parent sessions are refused' {
    Expect-Refusal { Invoke-OwnedProcess 'C:\other.exe' '/S' 'install' } 'unowned'
    $script:admin=$true
    Expect-Refusal { Invoke-OwnedProcess $state.setup '/S' 'install' } 'ordinary'
    if ($script:launches.Count) { throw 'Unsafe process launch.' }
}
Test-Guard 'running app is checked again at the process boundary' {
    $script:running=@(42)
    Expect-Refusal { Invoke-OwnedProcess $state.setup '/S' 'install' } 'Close PhotoForge yourself'
    if ($script:launches.Count) { throw 'Late app guard was bypassed.' }
}
Test-Guard 'only a new hash-checked owned uninstaller copy can execute' {
    $state.uninstallerHash='A' * 64
    $copy = Copy-OwnedUninstaller
    if ($copy -ne (Join-Path $RunDirectory 'owned-uninstaller.exe') -or $script:copies.Count -ne 1) { throw 'Wrong copy target.' }
    Expect-Refusal { Copy-OwnedUninstaller } 'copy already exists'
}
Test-Guard 'bad copied hash or elevated uninstaller is rejected' {
    $state.uninstallerHash='B' * 64
    Expect-Refusal { Copy-OwnedUninstaller } 'hash mismatch'
    $script:paths.Remove((Join-Path $RunDirectory 'owned-uninstaller.exe'))
    $state.uninstallerHash='A' * 64; $script:manifestLevel='requireAdministrator'
    Expect-Refusal { Copy-OwnedUninstaller } 'elevated uninstaller'
}
Test-Guard 'removal verifies absence but preserves the retained product key' {
    $script:paths[$userKey]=$true
    $state.productRegistry=Product-Snapshot
    Assert-Uninstalled
    if (-not $state.uninstallVerified -or -not $state.retainedProductRegistry -or -not $script:paths[$userKey]) { throw 'Retained registry state lost/misreported.' }
}
Test-Guard 'leftover payload or registration is not a removal pass' {
    $script:paths[$state.installDirectory]=$true
    Expect-Refusal { Assert-Uninstalled } 'files remain'
    $script:paths.Remove($state.installDirectory); $script:products=@(1)
    Expect-Refusal { Assert-Uninstalled } 'registration remains'
}
Test-Guard 'silent argument policy and absence of destructive automation are protected' {
    $strings=@($ast.FindAll({ param($node) $node -is [Management.Automation.Language.StringConstantExpressionAst] }, $true) | ForEach-Object Value)
    if ($strings -notcontains '/S /NS /D=' -or $strings -notcontains '/S /UPDATE _?=') { throw 'Silent arguments changed.' }
    $commands=@($ast.FindAll({ param($node) $node -is [Management.Automation.Language.CommandAst] }, $true) | ForEach-Object { $_.GetCommandName() })
    foreach ($forbidden in 'Stop-Process', 'Remove-Item', 'Set-Clipboard', 'Get-Clipboard', 'Invoke-Expression', 'node') {
        if ($commands -contains $forbidden) { throw "Forbidden automation: $forbidden" }
    }
    if ($ast.Extent.Text -match 'CloseMainWindow|(?:-Verb\s+RunAs)|cdp\.mjs|launch\.ps1') { throw 'GUI/force-close/elevation automation returned.' }
}
[pscustomobject]@{ tests=$script:passed; status='pass'; mockedBoundaries=$true;
    installerExecuted=$false; nativeLifecycleVerified=$false } | ConvertTo-Json
