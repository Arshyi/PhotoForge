# Tests backup serialization/integrity with synthetic data only. No clipboard API
# is invoked, no user data is read, and each run retains its small fixture directory.
$ErrorActionPreference = 'Stop'
$source = Get-Content -LiteralPath (Join-Path $PSScriptRoot 'clipboard_backup.ps1') -Raw
$definition = [regex]::Match($source, "(?s)Add-Type -TypeDefinition @'\r?\n(.*?)\r?\n'@")
if (-not $definition.Success) { throw 'Clipboard helper type definition not found.' }
Add-Type -TypeDefinition $definition.Groups[1].Value
$repoPath = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..')).ProviderPath
$fixtureRoot = Join-Path $repoPath ('.work\clipboard-backup-unit\run-' + [Guid]::NewGuid().ToString('N'))
$null = New-Item -ItemType Directory -Path $fixtureRoot
$script:passed = 0
function Fixture([string]$Name) {
    $directory = Join-Path $fixtureRoot $Name
    $items = New-Object 'System.Collections.Generic.List[PfClipItem]'
    $item = New-Object PfClipItem
    $item.Id = 13; $item.Name = 'CF_UNICODETEXT'; $item.Data = [Text.Encoding]::Unicode.GetBytes("synthetic backup fixture`0")
    $items.Add($item)
    [PfClip]::Write($directory, $items)
    $directory
}
function Expect-Refusal([scriptblock]$Body, [string]$Pattern) {
    try { & $Body } catch {
        if ($_.Exception.Message -notmatch $Pattern) { throw "Unexpected refusal: $($_.Exception.Message)" }
        return
    }
    throw "Expected refusal matching $Pattern"
}
function Test-Backup([string]$Name, [scriptblock]$Body) {
    & $Body
    $script:passed++
    Write-Output "PASS $Name"
}
Test-Backup 'synthetic backup round trip is byte-identical' {
    $directory = Fixture 'round-trip'
    $restored = [PfClip]::Read($directory)
    if ($restored.Count -ne 1 -or $restored[0].Id -ne 13 -or
        [Text.Encoding]::Unicode.GetString($restored[0].Data) -cne "synthetic backup fixture`0") { throw 'Round trip differs.' }
}
Test-Backup 'existing backups cannot be overwritten' {
    $directory = Fixture 'preserve'
    $path = Join-Path $directory 'format-13.bin'
    $hash = (Get-FileHash -LiteralPath $path).Hash
    $items = [PfClip]::Read($directory)
    Expect-Refusal { [PfClip]::Write($directory, $items) } 'Refusing to replace'
    if ((Get-FileHash -LiteralPath $path).Hash -ne $hash) { throw 'Existing backup changed.' }
}
Test-Backup 'same-size corruption is rejected before restoration' {
    $directory = Fixture 'tamper'
    $path = Join-Path $directory 'format-13.bin'
    $bytes = [IO.File]::ReadAllBytes($path); $bytes[0] = $bytes[0] -bxor 1
    [IO.File]::WriteAllBytes($path, $bytes)
    Expect-Refusal { [PfClip]::Read($directory) } 'checksum mismatch'
}
Test-Backup 'truncated data is rejected' {
    $directory = Fixture 'truncated'
    [IO.File]::WriteAllBytes((Join-Path $directory 'format-13.bin'), [byte[]]@(1,2))
    Expect-Refusal { [PfClip]::Read($directory) } 'length mismatch'
}
Test-Backup 'missing checksum is rejected' {
    $directory = Fixture 'no-checksum'
    $path = Join-Path $directory 'manifest.txt'
    $lines = @([IO.File]::ReadAllLines($path) | Where-Object { -not $_.StartsWith('sha256') })
    [IO.File]::WriteAllLines($path, $lines)
    Expect-Refusal { [PfClip]::Read($directory) } 'checksum mismatch'
}
Test-Backup 'manifest cannot redirect data outside its backup directory' {
    $directory = Fixture 'escape'
    $path = Join-Path $directory 'manifest.txt'
    [IO.File]::WriteAllText($path, [IO.File]::ReadAllText($path).Replace('format-13.bin', '..\outside.bin'))
    Expect-Refusal { [PfClip]::Read($directory) } 'filename does not match'
}
Test-Backup 'duplicate formats are rejected' {
    $directory = Fixture 'duplicate-format'
    $path = Join-Path $directory 'manifest.txt'
    $lines = [IO.File]::ReadAllLines($path)
    [IO.File]::AppendAllText($path, $lines[0] + [Environment]::NewLine)
    Expect-Refusal { [PfClip]::Read($directory) } 'duplicate clipboard format'
}
Test-Backup 'duplicate checksums are rejected' {
    $directory = Fixture 'duplicate-checksum'
    $path = Join-Path $directory 'manifest.txt'
    $lines = [IO.File]::ReadAllLines($path)
    [IO.File]::AppendAllText($path, $lines[-1] + [Environment]::NewLine)
    Expect-Refusal { [PfClip]::Read($directory) } 'Duplicate backup checksum'
}
Test-Backup 'malformed manifest is rejected' {
    $directory = Fixture 'malformed'
    $path = Join-Path $directory 'manifest.txt'
    [IO.File]::AppendAllText($path, 'unexpected record' + [Environment]::NewLine)
    Expect-Refusal { [PfClip]::Read($directory) } 'Malformed backup manifest'
}
[pscustomobject]@{ tests=$script:passed; status='pass'; clipboardTouched=$false } | ConvertTo-Json
