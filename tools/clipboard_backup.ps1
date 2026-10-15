<#
.SYNOPSIS
    Saves and restores the whole Windows clipboard, so a test that replaces it can give it back.

.DESCRIPTION
    Every format the clipboard holds in global memory (text in all its forms, bitmaps, HTML, file
    lists, registered formats such as "PNG") is written to a directory, in the order the clipboard
    lists them, and put back the same way. A format that cannot be saved is named, and the script
    **refuses** (exit 3) unless -AllowLoss says the loss is acceptable.

    What cannot be saved: formats that are GDI or metafile handles rather than global memory
    (CF_BITMAP, CF_PALETTE, CF_METAFILEPICT, CF_ENHMETAFILE and the owner-display and private
    ranges) and formats whose owner cannot render them. CF_BITMAP and CF_PALETTE are not a loss
    when a DIB is saved (Windows makes them from it); the same for the two metafile forms.

    The clipboard is the person's: a program that replaces it (tests/clipboard_real.rs,
    tools/clipboard_oracle.py, tools/validate/clipboard_window.ps1) saves it first with this and
    restores it after.

.PARAMETER Action
    save      write the clipboard to -Path (a directory; default <work>\clipboard-backup)
    restore   put what -Path holds back
    verify    compare the clipboard with what -Path holds (exit 0 identical, 1 different)
    list      print what the clipboard holds
    selftest  prove save/restore on a canary, then on the real clipboard, and leave it as it was

    Use a fresh backup directory for each save/selftest. Existing nonempty backups are
    never overwritten. Restoration checks manifest filenames, lengths and checksum first.

.EXAMPLE
    .\tools\clipboard_backup.ps1 save
    ... a test that replaces the clipboard ...
    .\tools\clipboard_backup.ps1 restore
    .\tools\clipboard_backup.ps1 verify
#>
param(
    [Parameter(Mandatory = $true)][ValidateSet('save', 'restore', 'verify', 'list', 'selftest')][string]$Action,
    [string]$Path = '',
    [switch]$AllowLoss
)
$ErrorActionPreference = 'Stop'
if (-not $Path) {
    $work = if ($env:PF_WORK) { $env:PF_WORK } else { Join-Path $PSScriptRoot '..\.work' }
    $Path = Join-Path $work 'clipboard-backup'
}
$Path = [System.IO.Path]::GetFullPath($Path)

Add-Type -TypeDefinition @'
using System;
using System.Collections.Generic;
using System.ComponentModel;
using System.IO;
using System.Runtime.InteropServices;
using System.Security.Cryptography;
using System.Text;
using System.Threading;

public class PfClipItem
{
    public uint Id;
    public string Name;
    public byte[] Data;      // null when not saved
    public string Skipped;   // why it was not saved, or null
    public bool Synthesized; // not saved, and not a loss: Windows makes it from another form
}

public static class PfClip
{
    [DllImport("user32.dll", SetLastError = true)] static extern bool OpenClipboard(IntPtr owner);
    [DllImport("user32.dll", SetLastError = true)] static extern bool CloseClipboard();
    [DllImport("user32.dll", SetLastError = true)] static extern bool EmptyClipboard();
    [DllImport("user32.dll")] static extern uint EnumClipboardFormats(uint format);
    [DllImport("user32.dll", SetLastError = true)] static extern IntPtr GetClipboardData(uint format);
    [DllImport("user32.dll", SetLastError = true)] static extern IntPtr SetClipboardData(uint format, IntPtr memory);
    [DllImport("user32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    static extern IntPtr CreateWindowEx(uint extendedStyle, string className, string name, uint style,
        int x, int y, int width, int height, IntPtr parent, IntPtr menu, IntPtr instance, IntPtr parameter);
    [DllImport("user32.dll", SetLastError = true)] static extern bool DestroyWindow(IntPtr window);
    [DllImport("kernel32.dll", CharSet = CharSet.Unicode)] static extern IntPtr GetModuleHandle(string name);
    [DllImport("user32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    static extern int GetClipboardFormatName(uint format, StringBuilder name, int max);
    [DllImport("user32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    static extern uint RegisterClipboardFormat(string name);
    [DllImport("kernel32.dll")] static extern IntPtr GlobalAlloc(uint flags, UIntPtr bytes);
    [DllImport("kernel32.dll")] static extern IntPtr GlobalFree(IntPtr memory);
    [DllImport("kernel32.dll")] static extern IntPtr GlobalLock(IntPtr memory);
    [DllImport("kernel32.dll")] static extern bool GlobalUnlock(IntPtr memory);
    [DllImport("kernel32.dll")] static extern UIntPtr GlobalSize(IntPtr memory);

    const uint CF_BITMAP = 2, CF_METAFILEPICT = 3, CF_DIB = 8, CF_PALETTE = 9, CF_ENHMETAFILE = 14,
        CF_DIBV5 = 17, CF_OWNERDISPLAY = 0x80, CF_DSPBITMAP = 0x82, CF_DSPMETAFILEPICT = 0x83,
        CF_DSPENHMETAFILE = 0x8E;
    const long MaxFormatBytes = 512L * 1024 * 1024;

    public static string FormatName(uint id)
    {
        switch (id)
        {
            case 1: return "CF_TEXT";
            case 2: return "CF_BITMAP";
            case 3: return "CF_METAFILEPICT";
            case 7: return "CF_OEMTEXT";
            case 8: return "CF_DIB";
            case 9: return "CF_PALETTE";
            case 13: return "CF_UNICODETEXT";
            case 14: return "CF_ENHMETAFILE";
            case 15: return "CF_HDROP";
            case 16: return "CF_LOCALE";
            case 17: return "CF_DIBV5";
        }
        StringBuilder name = new StringBuilder(256);
        int length = GetClipboardFormatName(id, name, name.Capacity);
        return length > 0 ? name.ToString() : "format " + id;
    }

    public static uint Register(string name) { return RegisterClipboardFormat(name); }

    static void Open(IntPtr owner = default(IntPtr))
    {
        for (int attempt = 0; attempt < 40; attempt++)
        {
            if (OpenClipboard(owner)) return;
            Thread.Sleep(50);
        }
        throw new InvalidOperationException("the clipboard is held by another program");
    }

    static bool IsHandleFormat(uint id)
    {
        return id == CF_BITMAP || id == CF_METAFILEPICT || id == CF_PALETTE || id == CF_ENHMETAFILE
            || id == CF_OWNERDISPLAY || id == CF_DSPBITMAP || id == CF_DSPMETAFILEPICT
            || id == CF_DSPENHMETAFILE || (id >= 0x200 && id <= 0x3FF);
    }

    /// <summary>Everything on the clipboard, in the clipboard's own order.</summary>
    public static List<PfClipItem> Snapshot()
    {
        List<PfClipItem> items = new List<PfClipItem>();
        Open();
        try
        {
            uint id = 0;
            while ((id = EnumClipboardFormats(id)) != 0)
            {
                PfClipItem item = new PfClipItem();
                item.Id = id;
                item.Name = FormatName(id);
                if (item.Name == "DataObject" || item.Name == "Ole Private Data")
                {
                    // Bookkeeping of the program that copied (an address in its process): not content,
                    // and meaningless, if not harmful, put back by another process.
                    item.Skipped = "OLE bookkeeping of the program that copied it";
                    item.Synthesized = true;
                }
                else if (IsHandleFormat(id))
                {
                    item.Skipped = "a handle format, not global memory";
                }
                else
                {
                    IntPtr handle = GetClipboardData(id);
                    if (handle == IntPtr.Zero)
                    {
                        item.Skipped = "its owner could not render it";
                    }
                    else
                    {
                        long size = (long)GlobalSize(handle).ToUInt64();
                        if (size <= 0)
                        {
                            item.Skipped = "not a global memory block";
                        }
                        else if (size > MaxFormatBytes)
                        {
                            item.Skipped = "larger than " + (MaxFormatBytes >> 20) + " MiB";
                        }
                        else
                        {
                            IntPtr pointer = GlobalLock(handle);
                            if (pointer == IntPtr.Zero)
                            {
                                item.Skipped = "could not be locked";
                            }
                            else
                            {
                                byte[] data = new byte[size];
                                Marshal.Copy(pointer, data, 0, (int)size);
                                GlobalUnlock(handle);
                                item.Data = data;
                            }
                        }
                    }
                }
                items.Add(item);
            }
        }
        finally
        {
            CloseClipboard();
        }
        bool haveDib = false, haveEmf = false, haveWmf = false;
        foreach (PfClipItem item in items)
        {
            if (item.Data == null) continue;
            if (item.Id == CF_DIB || item.Id == CF_DIBV5) haveDib = true;
            if (item.Id == CF_ENHMETAFILE) haveEmf = true;
            if (item.Id == CF_METAFILEPICT) haveWmf = true;
        }
        foreach (PfClipItem item in items)
        {
            if (item.Data != null) continue;
            bool made = ((item.Id == CF_BITMAP || item.Id == CF_PALETTE) && haveDib)
                || (item.Id == CF_ENHMETAFILE && haveWmf) || (item.Id == CF_METAFILEPICT && haveEmf);
            if (made)
            {
                item.Synthesized = true;
                item.Skipped = "made by Windows from another form";
            }
        }
        return items;
    }

    /// <summary>Replaces the clipboard with the items that carry data, in order.</summary>
    public static void Put(List<PfClipItem> items) { Put(items.ToArray()); }

    public static void Put(PfClipItem[] items)
    {
        // Memory first, so that a failure to allocate costs nothing that was on the clipboard.
        List<KeyValuePair<uint, IntPtr>> blocks = new List<KeyValuePair<uint, IntPtr>>();
        IntPtr owner = IntPtr.Zero;
        try
        {
            foreach (PfClipItem item in items)
            {
                if (item.Data == null) continue;
                IntPtr memory = GlobalAlloc(0x0002, new UIntPtr((uint)item.Data.Length));
                if (memory == IntPtr.Zero) throw new OutOfMemoryException("GlobalAlloc failed");
                IntPtr pointer = GlobalLock(memory);
                if (pointer == IntPtr.Zero)
                {
                    int error = Marshal.GetLastWin32Error();
                    GlobalFree(memory);
                    throw new Win32Exception(error, "GlobalLock failed before clipboard replacement");
                }
                Marshal.Copy(item.Data, 0, pointer, item.Data.Length);
                GlobalUnlock(memory);
                blocks.Add(new KeyValuePair<uint, IntPtr>(item.Id, memory));
            }
            // OpenClipboard(NULL) + EmptyClipboard leaves no owner: SetClipboardData can fail.
            // A message-only system-class window owns these immediate (not delayed) writes.
            owner = CreateWindowEx(0, "STATIC", "PhotoForge clipboard backup", 0,
                0, 0, 0, 0, new IntPtr(-3), IntPtr.Zero, GetModuleHandle(null), IntPtr.Zero);
            if (owner == IntPtr.Zero) throw new Win32Exception(Marshal.GetLastWin32Error(), "Cannot create clipboard owner");
            Open(owner);
            try
            {
                if (!EmptyClipboard()) throw new Win32Exception(Marshal.GetLastWin32Error(), "EmptyClipboard failed");
                for (int index = 0; index < blocks.Count; index++)
                {
                    if (SetClipboardData(blocks[index].Key, blocks[index].Value) == IntPtr.Zero)
                        throw new Win32Exception(Marshal.GetLastWin32Error(), "SetClipboardData failed for format " + blocks[index].Key);
                    // the clipboard owns it now
                    blocks[index] = new KeyValuePair<uint, IntPtr>(blocks[index].Key, IntPtr.Zero);
                }
            }
            finally
            {
                CloseClipboard();
            }
        }
        finally
        {
            if (owner != IntPtr.Zero) DestroyWindow(owner);
            foreach (KeyValuePair<uint, IntPtr> block in blocks)
            {
                if (block.Value != IntPtr.Zero) GlobalFree(block.Value);
            }
        }
    }

    public static void Empty()
    {
        Put(new PfClipItem[0]);
    }

    public static string Hash(List<PfClipItem> items)
    {
        using (SHA256 sha = SHA256.Create())
        {
            foreach (PfClipItem item in items)
            {
                if (item.Data == null) continue;
                byte[] id = BitConverter.GetBytes(item.Id);
                sha.TransformBlock(id, 0, id.Length, null, 0);
                sha.TransformBlock(item.Data, 0, item.Data.Length, null, 0);
            }
            sha.TransformFinalBlock(new byte[0], 0, 0);
            StringBuilder text = new StringBuilder();
            foreach (byte value in sha.Hash) text.Append(value.ToString("x2"));
            return text.ToString();
        }
    }

    public static void Write(string directory, List<PfClipItem> items)
    {
        if (Directory.Exists(directory))
        {
            if (Directory.GetFileSystemEntries(directory).Length != 0)
                throw new IOException("Refusing to replace an existing clipboard backup: " + directory);
        }
        Directory.CreateDirectory(directory);
        List<string> lines = new List<string>();
        foreach (PfClipItem item in items)
        {
            string file = "";
            string state = item.Skipped != null
                ? (item.Synthesized ? "synthesized" : "skipped: " + item.Skipped)
                : "saved";
            if (item.Data != null)
            {
                file = "format-" + item.Id + ".bin";
                File.WriteAllBytes(Path.Combine(directory, file), item.Data);
            }
            lines.Add(item.Id + "\t" + item.Name + "\t" + (item.Data == null ? 0 : item.Data.Length)
                + "\t" + file + "\t" + state);
        }
        lines.Add("sha256\t" + Hash(items));
        File.WriteAllLines(Path.Combine(directory, "manifest.txt"), lines.ToArray(), new UTF8Encoding(false));
    }

    public static List<PfClipItem> Read(string directory)
    {
        List<PfClipItem> items = new List<PfClipItem>();
        string manifest = Path.Combine(directory, "manifest.txt");
        if (!File.Exists(manifest)) throw new FileNotFoundException("no backup at " + directory);
        string expectedHash = null;
        HashSet<uint> formats = new HashSet<uint>();
        foreach (string line in File.ReadAllLines(manifest, Encoding.UTF8))
        {
            string[] fields = line.Split('\t');
            if (fields.Length == 2 && fields[0] == "sha256")
            {
                if (expectedHash != null) throw new InvalidDataException("Duplicate backup checksum");
                expectedHash = fields[1];
                continue;
            }
            if (fields.Length != 5) throw new InvalidDataException("Malformed backup manifest");
            PfClipItem item = new PfClipItem();
            item.Id = uint.Parse(fields[0]);
            if (item.Id == 0 || item.Id > 65535 || !formats.Add(item.Id))
                throw new InvalidDataException("Invalid or duplicate clipboard format");
            item.Name = fields[1];
            if (fields[3].Length > 0)
            {
                if (fields[3] != "format-" + item.Id + ".bin")
                    throw new InvalidDataException("Backup data filename does not match its format");
                item.Data = File.ReadAllBytes(Path.Combine(directory, fields[3]));
                if (item.Data.LongLength != long.Parse(fields[2]))
                    throw new InvalidDataException("Backup data length mismatch");
            }
            else item.Skipped = fields[4];
            item.Synthesized = fields[4] == "synthesized";
            items.Add(item);
        }
        if (expectedHash == null || !String.Equals(expectedHash, Hash(items), StringComparison.OrdinalIgnoreCase))
            throw new InvalidDataException("Backup checksum mismatch; clipboard will not be replaced");
        return items;
    }

    /// <summary>What is not the same between a backup and the clipboard now; empty if identical.</summary>
    public static List<string> Differences(List<PfClipItem> backup, List<PfClipItem> now)
    {
        List<string> problems = new List<string>();
        Dictionary<uint, PfClipItem> current = new Dictionary<uint, PfClipItem>();
        foreach (PfClipItem item in now) if (item.Data != null) current[item.Id] = item;
        Dictionary<uint, PfClipItem> wanted = new Dictionary<uint, PfClipItem>();
        foreach (PfClipItem item in backup) if (item.Data != null) wanted[item.Id] = item;
        foreach (KeyValuePair<uint, PfClipItem> pair in wanted)
        {
            PfClipItem other;
            if (!current.TryGetValue(pair.Key, out other)) { problems.Add("missing: " + pair.Value.Name); continue; }
            if (other.Data.Length != pair.Value.Data.Length) { problems.Add("size differs: " + pair.Value.Name); continue; }
            for (int i = 0; i < other.Data.Length; i++)
            {
                if (other.Data[i] != pair.Value.Data[i]) { problems.Add("bytes differ: " + pair.Value.Name); break; }
            }
        }
        foreach (KeyValuePair<uint, PfClipItem> pair in current)
        {
            if (!wanted.ContainsKey(pair.Key)) problems.Add("extra: " + pair.Value.Name);
        }
        return problems;
    }
}
'@

function Show($items) {
    if ($items.Count -eq 0) { '  (the clipboard is empty)'; return }
    foreach ($i in $items) {
        $n = if ($i.Data) { '{0,10:N0} bytes' -f $i.Data.Length } else { '               ' }
        $s = if ($i.Skipped) { if ($i.Synthesized) { "  [not restored, not content: $($i.Skipped)]" } else { "  [NOT SAVED: $($i.Skipped)]" } } else { '' }
        '  {0,6}  {1,-34} {2}{3}' -f $i.Id, $i.Name, $n, $s
    }
}
function Lost($items) { @($items | Where-Object { $_.Skipped -and -not $_.Synthesized }) }
function Plain([string]$text) { [System.Text.Encoding]::Unicode.GetBytes($text + [char]0) }
function NewItem($id, [byte[]]$data) { $o = New-Object PfClipItem; $o.Id = $id; $o.Name = [PfClip]::FormatName($id); $o.Data = $data; $o }

switch ($Action) {
    'list' {
        Show ([PfClip]::Snapshot())
    }
    'save' {
        $items = [PfClip]::Snapshot()
        Show $items
        $lost = Lost $items
        if ($lost.Count -gt 0 -and -not $AllowLoss) {
            Write-Output "REFUSED: $($lost.Count) format(s) cannot be saved, so the clipboard could not be given back. Copy something else first, or pass -AllowLoss."
            exit 3
        }
        [PfClip]::Write($Path, $items)
        $saved = @($items | Where-Object { $_.Data }).Count
        Write-Output "saved $saved format(s) to $Path"
        if ($lost.Count -gt 0) { Write-Output "(not saved: $(($lost | ForEach-Object { $_.Name }) -join ', '))" }
    }
    'restore' {
        $items = [PfClip]::Read($Path)
        [PfClip]::Put($items)
        Write-Output "restored $(@($items | Where-Object { $_.Data }).Count) format(s)"
    }
    'verify' {
        $backup = [PfClip]::Read($Path)
        $problems = [PfClip]::Differences($backup, [PfClip]::Snapshot())
        if ($problems.Count -eq 0) { Write-Output 'identical'; exit 0 }
        $problems | ForEach-Object { Write-Output $_ }
        exit 1
    }
    'selftest' {
        # 1. the real clipboard, saved first; refused if it cannot be given back
        $original = Join-Path $Path 'original'
        $items = [PfClip]::Snapshot()
        'the clipboard holds:'; Show $items
        $lost = Lost $items
        if ($lost.Count -gt 0 -and -not $AllowLoss) {
            Write-Output "REFUSED: $($lost.Count) format(s) cannot be saved. Nothing was touched."
            exit 3
        }
        [PfClip]::Write($original, $items)
        $failed = $false
        try {
            # 2. a canary with the kinds of format that matter: Unicode text, a DIB, a registered one
            $png = [PfClip]::Register('PNG')
            $private = [PfClip]::Register('PhotoForgeCanary')
            $dib = [byte[]](40,0,0,0, 2,0,0,0, 2,0,0,0, 1,0, 24,0, 0,0,0,0, 16,0,0,0, 0,0,0,0, 0,0,0,0, 0,0,0,0, 0,0,0,0,
                            0,0,255,0, 0,255,0,0,  255,0,0,0, 255,255,255,0)
            $bytes = New-Object byte[] 4096; (New-Object System.Random 7).NextBytes($bytes)
            $canary = @(
                (NewItem 13 (Plain ('photoforge clipboard canary: ' + [char]0xFC + [char]0x20AC + [char]0x65E5))),
                (NewItem 8 $dib),
                (NewItem $png $bytes),
                (NewItem $private $bytes)
            )
            [PfClip]::Put([PfClipItem[]]$canary)
            $before = [PfClip]::Snapshot()
            [PfClip]::Write((Join-Path $Path 'canary'), $before)
            # 3. replace it with something else, put the canary back, compare
            [PfClip]::Put([PfClipItem[]]@((NewItem 13 (Plain 'something else'))))
            [PfClip]::Put([PfClip]::Read((Join-Path $Path 'canary')))
            $problems = [PfClip]::Differences($before, [PfClip]::Snapshot())
            if ($problems.Count -eq 0) { 'canary round trip: identical' }
            else { $failed = $true; "canary round trip: DIFFERENT ($($problems -join '; '))" }
        }
        finally {
            # 4. the person's clipboard, back exactly as it was
            [PfClip]::Put([PfClip]::Read($original))
        }
        $problems = [PfClip]::Differences([PfClip]::Read($original), [PfClip]::Snapshot())
        if ($problems.Count -eq 0) { 'your clipboard: restored, identical' }
        else { $failed = $true; "your clipboard: DIFFERENT ($($problems -join '; '))" }
        if ($failed) { exit 1 }
    }
}
