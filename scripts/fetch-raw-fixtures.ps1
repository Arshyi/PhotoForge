<#
.SYNOPSIS
    Downloads the public-domain camera RAW files PhotoForge's optional
    real-file decoder tests use.

.DESCRIPTION
    These files are tens of megabytes each, so they are not committed to the
    repository. Nothing in the normal test run touches the network: the tests
    in src-tauri/tests/raw_real_files.rs skip themselves unless
    PHOTOFORGE_RAW_FIXTURES names a directory containing these files.

    Run this once, by hand, when you want to exercise the decoder against real
    camera output.

    Source: https://raw.pixls.us, the sample repository darktable and
    RawTherapee use. Every file there is released under Creative Commons Zero
    (public domain dedication), so redistributing and testing against them is
    unencumbered.

.PARAMETER Destination
    Where to put the files. Defaults to a raw-fixtures directory beside the
    repository, which is git-ignored.

.EXAMPLE
    ./scripts/fetch-raw-fixtures.ps1
    $env:PHOTOFORGE_RAW_FIXTURES = "$PWD/raw-fixtures"
    cargo test --manifest-path src-tauri/Cargo.toml --test raw_real_files
#>
[CmdletBinding()]
param(
    [string]$Destination = (Join-Path $PSScriptRoot ".." "raw-fixtures")
)

$ErrorActionPreference = "Stop"

# Canon EOS 5D Mark III, written by Adobe DNG Converter in three encodings of
# the same photograph. The uncompressed and lossless pair is what proves the
# entropy decoder exact; the lossy one is what proves it is refused.
$files = @(
    @{ Name = "5G4A9394-compressed-lossless.DNG"; Bytes = 23206642 }
    @{ Name = "5G4A9394-uncompressed.DNG"; Bytes = 48382640 }
    @{ Name = "5G4A9394-compressed-lossy.DNG"; Bytes = 6193902 }
)
$base = "https://raw.pixls.us/data/Adobe%20DNG%20Converter/Canon%20EOS%205D%20Mark%20III"

$Destination = [System.IO.Path]::GetFullPath($Destination)
New-Item -ItemType Directory -Force -Path $Destination | Out-Null
Write-Host "Fetching CC0 RAW fixtures into $Destination"
Write-Host "Total download: about 75 MB."

foreach ($file in $files) {
    $target = Join-Path $Destination $file.Name
    if ((Test-Path $target) -and ((Get-Item $target).Length -eq $file.Bytes)) {
        Write-Host "  already present: $($file.Name)"
        continue
    }
    Write-Host "  downloading $($file.Name) ..."
    Invoke-WebRequest -Uri "$base/$($file.Name)" -OutFile $target -UseBasicParsing
    $actual = (Get-Item $target).Length
    if ($actual -ne $file.Bytes) {
        Write-Warning "  $($file.Name) is $actual bytes, expected $($file.Bytes). The upstream sample may have been replaced."
    }
}

Write-Host ""
Write-Host "Done. To run the optional real-file tests:"
Write-Host "  `$env:PHOTOFORGE_RAW_FIXTURES = `"$Destination`""
Write-Host "  cargo test --manifest-path src-tauri/Cargo.toml --test raw_real_files"
