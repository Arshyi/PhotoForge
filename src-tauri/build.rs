fn main() {
    // Test binaries carry no application manifest, so they load the v5 side of
    // comctl32 and fail at startup on the v6-only exports (`TaskDialogIndirect`,
    // `RemoveWindowSubclass`, `DefSubclassProc`) that Tauri's windowing layer
    // imports. The shipped executable gets this dependency from the bundle's own
    // manifest; asking the linker for it on test targets only lets
    // `cargo test` load the same code without touching the release build.
    #[cfg(windows)]
    println!(
        "cargo:rustc-link-arg-tests=/MANIFESTDEPENDENCY:type='win32' \
         name='Microsoft.Windows.Common-Controls' version='6.0.0.0' \
         processorArchitecture='*' publicKeyToken='6595b64144ccf1df' language='*'"
    );
    tauri_build::build()
}
