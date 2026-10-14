# Read-only MSI database policy inspection. Does not install, extract or invoke custom actions.
param(
    [Parameter(Mandatory)][string]$Msi,
    [string]$ExpectedVersion = ''
)
$ErrorActionPreference = 'Stop'
if (-not $IsWindows -and $PSVersionTable.PSEdition -eq 'Core') { throw 'Windows Installer inspection requires Windows.' }
$msiPath = (Resolve-Path -LiteralPath $Msi).ProviderPath
if (-not $ExpectedVersion) {
    $repoPath = (Resolve-Path (Join-Path $PSScriptRoot '..\..')).ProviderPath
    $ExpectedVersion = (Get-Content -LiteralPath (Join-Path $repoPath 'src-tauri\tauri.conf.json') -Raw | ConvertFrom-Json).version
}
function Assert-Policy($condition, $message) { if (-not $condition) { throw "MSI policy: $message" } }
$installer = New-Object -ComObject WindowsInstaller.Installer
$database = $null
try {
    $database = $installer.OpenDatabase($msiPath, 0)
    function Read-Rows([string]$sql, [string[]]$columns) {
        $view = $database.OpenView($sql)
        try {
            $null = $view.Execute()
            while ($record = $view.Fetch()) {
                try {
                    $row = [ordered]@{}
                    for ($index = 0; $index -lt $columns.Count; $index++) {
                        $row[$columns[$index]] = $record.StringData($index + 1)
                    }
                    [pscustomobject]$row
                } finally { $null = [Runtime.InteropServices.Marshal]::FinalReleaseComObject($record) }
            }
        } finally {
            $null = $view.Close()
            $null = [Runtime.InteropServices.Marshal]::FinalReleaseComObject($view)
        }
    }
    $properties = @{}
    foreach ($row in Read-Rows 'SELECT `Property`, `Value` FROM `Property`' @('Property','Value')) {
        $properties[$row.Property] = $row.Value
    }
    Assert-Policy ($properties.ProductVersion -eq $ExpectedVersion) 'Unexpected product version.'
    Assert-Policy ($properties.ALLUSERS -eq '1') 'ALLUSERS must be 1.'
    Assert-Policy ($properties.UpgradeCode -ieq '{DA34C5F7-E5BB-583B-93F8-1F4E4065DC14}') 'The existing upgrade identity changed.'
    Assert-Policy (@($properties.SecureCustomProperties -split ';') -contains 'INSTALLDIR') 'INSTALLDIR is not secure.'

    $searches = @(Read-Rows 'SELECT `Property`, `Signature_` FROM `AppSearch`' @('Property','Signature'))
    $locators = @(Read-Rows 'SELECT `Signature_`, `Root`, `Key`, `Name`, `Type` FROM `RegLocator`' @('Signature','Root','Key','Name','Type'))
    $installSearch = @($searches | Where-Object Property -eq 'INSTALLDIR')
    Assert-Policy ($installSearch.Count -eq 1) 'INSTALLDIR must have one search.'
    $installLocator = @($locators | Where-Object Signature -eq $installSearch[0].Signature)
    Assert-Policy ($installLocator.Count -eq 1) 'INSTALLDIR has no unique registry locator.'
    Assert-Policy ($installLocator[0].Root -eq '2' -and $installLocator[0].Name -eq 'InstallDir') 'INSTALLDIR still uses a per-user/NSIS path.'
    Assert-Policy (([int]$installLocator[0].Type -band 16) -ne 0) 'INSTALLDIR must read the native 64-bit registry view.'

    $directories = @(Read-Rows 'SELECT `Directory`, `Directory_Parent`, `DefaultDir` FROM `Directory`' @('Id','Parent','Name'))
    $installDirectory = @($directories | Where-Object Id -eq 'INSTALLDIR')
    Assert-Policy ($installDirectory.Count -eq 1 -and $installDirectory[0].Parent -eq 'ProgramFiles64Folder') 'The x64 install default must be Program Files, not a user profile.'
    $registry = @(Read-Rows 'SELECT `Root`, `Key`, `Name`, `Component_` FROM `Registry`' @('Root','Key','Name','Component'))
    $productRegistry = @($registry | Where-Object Key -eq 'Software\photoforge\PhotoForge')
    Assert-Policy ($productRegistry.Count -ge 4) 'Expected machine product/shortcut registration is missing.'
    Assert-Policy (@($productRegistry | Where-Object Root -ne '2').Count -eq 0) 'Product registration writes the current-user hive.'
    $components = @(Read-Rows 'SELECT `Component`, `Attributes` FROM `Component`' @('Id','Attributes'))
    foreach ($entry in $productRegistry) {
        $component = @($components | Where-Object Id -eq $entry.Component)
        Assert-Policy ($component.Count -eq 1 -and (([int]$component[0].Attributes -band 256) -ne 0)) "Wrong registry view for component $($entry.Component)."
    }
    $upgrades = @(Read-Rows 'SELECT `UpgradeCode`, `VersionMax`, `Attributes`, `ActionProperty` FROM `Upgrade`' @('Code','Maximum','Attributes','Property'))
    $legacy = @($upgrades | Where-Object Property -eq 'PF_LEGACY_MSI')
    Assert-Policy ($legacy.Count -eq 1 -and $legacy[0].Maximum -eq '0.15.1' -and
        (([int]$legacy[0].Attributes -band 2) -ne 0) -and (([int]$legacy[0].Attributes -band 512) -ne 0)) 'Legacy migration must be detect-only and include 0.15.1.'
    $conditions = @(Read-Rows 'SELECT `Condition`, `Description` FROM `LaunchCondition`' @('Condition','Message'))
    Assert-Policy (@($conditions | Where-Object Condition -match 'NOT PF_LEGACY_MSI').Count -eq 1) 'Legacy migration is not refused before execution.'
    $removals = @(Read-Rows 'SELECT `FileName`, `DirProperty` FROM `RemoveFile`' @('Name','Directory'))
    Assert-Policy (@($removals | Where-Object { $_.Directory -eq 'DesktopFolder' -and -not $_.Name }).Count -eq 0) 'Uninstall must not remove the shared Desktop directory.'
    $files = @(Read-Rows 'SELECT `FileName` FROM `File`' @('Name'))
    foreach ($required in 'photoforge.exe','THIRD_PARTY_NOTICES.md','Tauri-MIT.txt') {
        Assert-Policy (@($files | Where-Object { ($_.Name -split '\|')[-1] -eq $required }).Count -eq 1) "Missing payload: $required"
    }
    [pscustomobject]@{
        status='pass'; msi=$msiPath; version=$properties.ProductVersion;
        productCode=$properties.ProductCode; scope='perMachine'; installDirectoryParent=$installDirectory[0].Parent;
        machineRegistryEntries=$productRegistry.Count; legacyMigration='explicit uninstall required through 0.15.1';
        nativeLifecycleVerified=$false
    } | ConvertTo-Json -Depth 4
} finally {
    if ($database) { $null = [Runtime.InteropServices.Marshal]::FinalReleaseComObject($database) }
    $null = [Runtime.InteropServices.Marshal]::FinalReleaseComObject($installer)
}
