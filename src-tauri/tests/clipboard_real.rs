//! The clipboard code against the real Windows clipboard.
//!
//! **These replace whatever is on the clipboard**, so they do nothing unless
//! `PHOTOFORGE_CLIPBOARD_TESTS=1` is set. They serialise on one lock (the clipboard is global),
//! and the "another program" cases use Windows PowerShell and System.Windows.Forms as an
//! independent writer and reader of the same clipboard.
//!
//! Without the variable each test prints that it is skipping and passes.
#![cfg(windows)]
use image::{Rgba, RgbaImage};
use photoforge_lib::clipboard::{
    has_image, read_image, write_image, ClipboardError, ClipboardFormat,
};
use std::process::Command;
use std::sync::Mutex;

static CLIPBOARD: Mutex<()> = Mutex::new(());

fn enabled() -> bool {
    std::env::var("PHOTOFORGE_CLIPBOARD_TESTS").is_ok_and(|v| v == "1")
}

macro_rules! real_clipboard {
    () => {
        if !enabled() {
            eprintln!("PHOTOFORGE_CLIPBOARD_TESTS is not 1: this test would replace the clipboard, so it does not run");
            return;
        }
        let _lock = CLIPBOARD.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    };
}

fn picture() -> RgbaImage {
    let mut image = RgbaImage::new(6, 4);
    for (x, y, pixel) in image.enumerate_pixels_mut() {
        *pixel = Rgba([
            (x * 40) as u8,
            (y * 60) as u8,
            (255 - x * 30 - y * 20) as u8,
            if (x + y) % 3 == 0 {
                255
            } else {
                (60 + x * 30) as u8
            },
        ]);
    }
    image
}

fn powershell(script: &str) -> String {
    let output = Command::new("powershell.exe")
        .args(["-STA", "-NoProfile", "-NonInteractive", "-Command", script])
        .output()
        .expect("Windows PowerShell runs");
    assert!(
        output.status.success(),
        "powershell failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

#[test]
fn a_picture_written_is_the_picture_read_with_its_alpha() {
    real_clipboard!();
    let image = picture();
    let written = write_image(&image).unwrap();
    assert_eq!((written.width, written.height), (6, 4));
    assert_eq!(written.formats, ["PNG", "CF_DIBV5"]);
    assert!(has_image());
    let pasted = read_image(1 << 20).unwrap();
    assert_eq!(pasted.format, ClipboardFormat::Png);
    assert_eq!(pasted.image, image);
}

#[test]
fn a_picture_is_replaced_whole_by_the_next() {
    real_clipboard!();
    write_image(&picture()).unwrap();
    let mut other = RgbaImage::new(2, 2);
    other.put_pixel(1, 1, Rgba([1, 2, 3, 255]));
    write_image(&other).unwrap();
    assert_eq!(read_image(1 << 20).unwrap().image, other);
}

#[test]
fn a_larger_picture_than_the_limit_is_refused_not_decoded() {
    real_clipboard!();
    write_image(&picture()).unwrap();
    assert!(matches!(
        read_image(10),
        Err(ClipboardError::TooLarge { .. })
    ));
}

#[test]
fn text_on_the_clipboard_is_not_a_picture() {
    real_clipboard!();
    powershell("Set-Clipboard -Value 'just some words'");
    assert!(!has_image());
    assert_eq!(read_image(1 << 20).unwrap_err(), ClipboardError::NoImage);
}

#[test]
fn a_picture_another_program_copied_is_read() {
    real_clipboard!();
    // System.Windows.Forms puts a bitmap on the clipboard the way most Windows programs do (a
    // device-dependent bitmap, from which Windows synthesises the DIB forms). Opaque colours.
    powershell(
        "Add-Type -AssemblyName System.Windows.Forms, System.Drawing; \
         $b = New-Object System.Drawing.Bitmap 5, 3; \
         for ($x = 0; $x -lt 5; $x++) { for ($y = 0; $y -lt 3; $y++) { \
           $b.SetPixel($x, $y, [System.Drawing.Color]::FromArgb(255, $x * 50, $y * 80, 200)) } }; \
         [System.Windows.Forms.Clipboard]::SetImage($b)",
    );
    assert!(has_image());
    let pasted = read_image(1 << 20).unwrap();
    assert_eq!(pasted.image.dimensions(), (5, 3));
    for y in 0..3_u32 {
        for x in 0..5_u32 {
            assert_eq!(
                pasted.image.get_pixel(x, y).0,
                [(x * 50) as u8, (y * 80) as u8, 200, 255],
                "pixel {x},{y} read as {:?} from {:?}",
                pasted.image.get_pixel(x, y),
                pasted.format
            );
        }
    }
}

#[test]
fn what_is_written_is_read_by_another_program() {
    real_clipboard!();
    // An opaque picture, so that the check does not depend on how a reader treats alpha.
    let mut image = RgbaImage::new(4, 3);
    for (x, y, pixel) in image.enumerate_pixels_mut() {
        *pixel = Rgba([(x * 60) as u8, (y * 90) as u8, 120, 255]);
    }
    write_image(&image).unwrap();
    let report = powershell(
        "Add-Type -AssemblyName System.Windows.Forms, System.Drawing; \
         $i = [System.Windows.Forms.Clipboard]::GetImage(); \
         if ($i -eq $null) { 'none' } else { \
           $o = @(\"$($i.Width)x$($i.Height)\"); \
           for ($y = 0; $y -lt $i.Height; $y++) { for ($x = 0; $x -lt $i.Width; $x++) { \
             $c = $i.GetPixel($x, $y); $o += \"$($c.R),$($c.G),$($c.B),$($c.A)\" } }; \
           $o -join ';' }",
    );
    assert_ne!(
        report, "none",
        "System.Windows.Forms found no image on the clipboard"
    );
    let mut parts = report.split(';');
    assert_eq!(parts.next(), Some("4x3"));
    let seen: Vec<[u8; 4]> = parts
        .map(|p| {
            let v: Vec<u8> = p.split(',').map(|n| n.parse().unwrap()).collect();
            [v[0], v[1], v[2], v[3]]
        })
        .collect();
    assert_eq!(seen.len(), 12);
    for (index, pixel) in seen.iter().enumerate() {
        let (x, y) = ((index % 4) as u32, (index / 4) as u32);
        assert_eq!(*pixel, image.get_pixel(x, y).0, "pixel {x},{y}");
    }
}

#[test]
fn a_dibv5_alone_is_read_by_windows_itself() {
    // A clipboard with a PNG form would hide the DIBV5 layout from a reader that prefers PNG, so
    // the picture is put there as CF_DIBV5 alone and Windows' own reader (through System.Drawing)
    // is asked what it makes of it.
    real_clipboard!();
    let mut image = RgbaImage::new(4, 3);
    for (x, y, pixel) in image.enumerate_pixels_mut() {
        *pixel = Rgba([(x * 60) as u8, (y * 90) as u8, 120, 255]);
    }
    photoforge_lib::clipboard::write_dibv5_only(&image, false).unwrap();
    let report = powershell(
        "Add-Type -AssemblyName System.Windows.Forms, System.Drawing;          $i = [System.Windows.Forms.Clipboard]::GetImage();          if ($i -eq $null) { 'none' } else {            $o = @(\"$($i.Width)x$($i.Height)\");            for ($y = 0; $y -lt $i.Height; $y++) { for ($x = 0; $x -lt $i.Width; $x++) {              $c = $i.GetPixel($x, $y); $o += \"$($c.R),$($c.G),$($c.B),$($c.A)\" } };            $o -join ';' }",
    );
    assert_ne!(report, "none");
    let mut parts = report.split(';');
    assert_eq!(parts.next(), Some("4x3"));
    for (index, text) in parts.enumerate() {
        let v: Vec<u8> = text.split(',').map(|n| n.parse().unwrap()).collect();
        let (x, y) = ((index % 4) as u32, (index / 4) as u32);
        assert_eq!(v, image.get_pixel(x, y).0, "pixel {x},{y}");
    }
    // And this program reads its own DIBV5 back, with or without the mask table Windows adds.
    for mask_table in [false, true] {
        photoforge_lib::clipboard::write_dibv5_only(&image, mask_table).unwrap();
        let read = read_image(1 << 20).unwrap();
        assert_eq!(read.format, ClipboardFormat::DibV5);
        assert_eq!(read.image, image, "mask table: {mask_table}");
    }
}

/// A step for an outside oracle (`tools/clipboard_oracle.py` in the scratch area): leaves the sample
/// picture, with alpha, on the clipboard as CF_DIBV5 alone, for another program to read.
#[test]
fn step_leave_a_dibv5_with_alpha_on_the_clipboard() {
    if std::env::var("PHOTOFORGE_CLIPBOARD_STEP").as_deref() != Ok("dibv5_alpha") {
        return;
    }
    real_clipboard!();
    photoforge_lib::clipboard::write_dibv5_only(&picture(), false).unwrap();
}

/// A step for an outside oracle: leaves the sample picture on the clipboard in both forms.
#[test]
fn step_leave_the_sample_on_the_clipboard() {
    if std::env::var("PHOTOFORGE_CLIPBOARD_STEP").as_deref() != Ok("both") {
        return;
    }
    real_clipboard!();
    write_image(&picture()).unwrap();
}

/// A step for an outside oracle: reads the clipboard with this program's reader and writes what
/// it found (the form, the size, every pixel) as JSON to the path in `PHOTOFORGE_CLIPBOARD_REPORT`.
#[test]
fn step_report_what_is_on_the_clipboard() {
    if std::env::var("PHOTOFORGE_CLIPBOARD_STEP").as_deref() != Ok("report") {
        return;
    }
    real_clipboard!();
    let path = std::env::var("PHOTOFORGE_CLIPBOARD_REPORT").expect("a report path");
    let report = match read_image(1 << 24) {
        Ok(pasted) => {
            let (width, height) = pasted.image.dimensions();
            let pixels: Vec<[u8; 4]> = pasted.image.pixels().map(|p| p.0).collect();
            format!(
                "{{\"format\":\"{:?}\",\"width\":{width},\"height\":{height},\"pixels\":{pixels:?}}}",
                pasted.format
            )
        }
        Err(error) => format!("{{\"error\":\"{error}\"}}"),
    };
    std::fs::write(path, report).unwrap();
}

#[test]
fn a_copy_replaces_what_was_there_and_does_not_add_to_it() {
    real_clipboard!();
    // Text first; then a picture. The text must be gone, not left beside the picture.
    powershell("Set-Clipboard -Value 'words that should not survive a copy'");
    let before = powershell(
        "Add-Type -AssemblyName System.Windows.Forms; \
         [System.Windows.Forms.Clipboard]::ContainsText()",
    );
    assert_eq!(before, "True");
    write_image(&picture()).unwrap();
    let after = powershell(
        "Add-Type -AssemblyName System.Windows.Forms; \
         [System.Windows.Forms.Clipboard]::ContainsText()",
    );
    assert_eq!(
        after, "False",
        "the text is still on the clipboard beside the picture"
    );
    assert!(has_image());
}

#[test]
fn a_clipboard_another_program_holds_is_busy_not_a_hang() {
    real_clipboard!();
    write_image(&picture()).unwrap();
    let flag =
        std::env::temp_dir().join(format!("photoforge-clipboard-held-{}", std::process::id()));
    let _ = std::fs::remove_file(&flag);
    // Another program opens the clipboard, as a program with a window does, and keeps it for a
    // few seconds. (A program that opens it with no window is not excluded by Windows at all.)
    let script = format!(
        "Add-Type -AssemblyName System.Windows.Forms; \
         Add-Type -Namespace W -Name C -MemberDefinition \
         '[DllImport(\"user32.dll\")] public static extern bool OpenClipboard(System.IntPtr h); \
          [DllImport(\"user32.dll\")] public static extern bool CloseClipboard();'; \
         $form = New-Object System.Windows.Forms.Form; \
         $opened = [W.C]::OpenClipboard($form.Handle); \
         Set-Content -Path '{}' -Value \"opened=$opened\"; \
         Start-Sleep -Milliseconds 3500; \
         [void][W.C]::CloseClipboard()",
        flag.display()
    );
    let mut holder = Command::new("powershell.exe")
        .args(["-STA", "-NoProfile", "-NonInteractive", "-Command", &script])
        .spawn()
        .expect("Windows PowerShell runs");
    let started = std::time::Instant::now();
    while !flag.exists() {
        assert!(
            started.elapsed().as_secs() < 30,
            "the other program never opened the clipboard"
        );
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    let said = std::fs::read_to_string(&flag).unwrap_or_default();
    assert!(
        said.contains("opened=True"),
        "the other program said: {said}"
    );
    // While it is held: a copy says so, quickly, and a check for a picture says no.
    let asked = std::time::Instant::now();
    let result = write_image(&picture());
    assert_eq!(result.unwrap_err(), ClipboardError::Busy);
    assert!(
        asked.elapsed().as_secs() < 3,
        "waited {:?}",
        asked.elapsed()
    );
    assert!(!has_image());
    // Once it lets go, both work again.
    holder.wait().unwrap();
    let _ = std::fs::remove_file(&flag);
    write_image(&picture()).unwrap();
    assert!(has_image());
}
