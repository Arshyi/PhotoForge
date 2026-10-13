param([string]$Action, [string]$Path = '')
if (-not $Path) {
    $work = if ($env:PF_WORK) { $env:PF_WORK } else { Join-Path $PSScriptRoot '..\.work' }
    New-Item -ItemType Directory -Force $work | Out-Null
    $Path = Join-Path $work 'clipboard-backup.png'
}
Add-Type -AssemblyName System.Windows.Forms, System.Drawing
if ($Action -eq 'save') {
    $d = [System.Windows.Forms.Clipboard]::GetDataObject()
    $saved = $false
    if ($d -ne $null -and $d.GetDataPresent('PNG')) {
        $s = $d.GetData('PNG')
        if ($s -is [System.IO.MemoryStream]) {
            [System.IO.File]::WriteAllBytes($Path, $s.ToArray())
            $saved = $true
        }
    }
    if (-not $saved -and [System.Windows.Forms.Clipboard]::ContainsImage()) {
        $i = [System.Windows.Forms.Clipboard]::GetImage()
        $i.Save($Path, [System.Drawing.Imaging.ImageFormat]::Png)
        $saved = $true
    }
    if ($saved) { "saved $((Get-Item $Path).Length) bytes" } else { 'nothing to save' }
}
elseif ($Action -eq 'restore') {
    if (Test-Path $Path) {
        $bytes = [System.IO.File]::ReadAllBytes($Path)
        $ms = New-Object System.IO.MemoryStream (, $bytes)
        $img = [System.Drawing.Image]::FromStream($ms)
        $o = New-Object System.Windows.Forms.DataObject
        $o.SetData('PNG', (New-Object System.IO.MemoryStream (, $bytes)))
        $o.SetImage($img)
        [System.Windows.Forms.Clipboard]::SetDataObject($o, $true)
        'restored'
    } else { 'no backup' }
}
