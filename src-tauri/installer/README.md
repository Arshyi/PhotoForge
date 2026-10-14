# Per-machine MSI template

Adapted from the official Tauri CLI `tauri-cli-v2.11.4` template:
https://github.com/tauri-apps/tauri/blob/tauri-cli-v2.11.4/crates/tauri-bundler/src/bundle/windows/msi/main.wxs

Original UTF-8 template SHA-256: `e371a01628a06730828f9bd24111feacb8bec53c250ccec4b46df756fe0a0198`.
The upstream MIT notice is retained in `Tauri-MIT.txt`, also bundled as a third-party notice.
This does not assign a license to PhotoForge; its project license remains unset by user choice.

Changes from that template:

- INSTALLDIR is a secure property and reads only the native-view HKLM InstallDir value.
  It never inherits the NSIS HKCU default value or the legacy MSI HKCU InstallDir value.
- Product/uninstall registration uses HKLM and the native architecture view. Desktop and
  Start-menu shortcuts are advertised links owned by the required application file component,
  not profile-directory components with incompatible machine registry key paths. The executable
  belongs only to the mandatory MainProgram feature; it cannot disappear with an optional
  shortcut/PATH feature. Both application shortcuts remain included, with the proper .exe icon
  identity. Per-user NSIS registration remains separate.
- Legacy MSI versions through 0.15.1 are detected and refused for automatic major upgrade:
  their uninstaller could remove an overlapping per-user NSIS installation. Migration must
  be explicit rather than silently running the old unsafe uninstall.
- Uninstall removes its desktop shortcut, not the shared system Desktop directory.
- The stable upgrade code, resource handling, file associations, optional integrations and
  upstream WebView2 handling are retained. WebView2's upstream user-runtime detection is
  not proof of availability for every Windows account; multi-account prerequisites remain
  an acceptance concern, not a claimed result.

When upgrading Tauri, compare the pinned upstream template before adapting it. Policy tests
check the source structure; built-MSI table inspection and real lifecycle testing are still
required. No administrative extraction or template check substitutes for UAC lifecycle proof.

The first HKLM-only shortcut-component draft failed actual WiX ICE38/43/57 checks. Moving
the links to the required file component and matching their advertised icon identity fixed
those errors (including ICE50), without suppressing validation. The resulting MSI passed the
read-only table inspector. Upstream ICE03 (bootstrapper command length), ICE40 (REINSTALLMODE)
and ICE61 (same-version upgrades) warnings remain recorded; no zero-warning build is claimed.
