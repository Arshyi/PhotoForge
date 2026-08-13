# WebView2 network boundary

PhotoForge does not rely on remote web content. Its Windows shell nevertheless uses the Microsoft Edge WebView2 Runtime, which is an operating-system component with networking behavior outside PhotoForge's renderer and Rust request paths. This document separates the controls PhotoForge can enforce from runtime traffic it cannot promise to suppress.

## Enforced application boundary

The main window is created in Rust instead of being auto-created from configuration. Startup is deliberately sequenced as follows:

1. WebView2 is created on a script-free inert bundled document at the normal Tauri application origin, preserving Tauri's per-webview origin metadata.
2. On Windows, PhotoForge uses WebView2's documented `IsReputationCheckingRequired` setting to disable SmartScreen and reads the value back. Startup fails if the setting cannot be applied or verified.
3. PhotoForge navigates to its exact bundled Tauri origin. Release builds do not allow the configured development-server origin.

The window then enforces these independent controls:

- top-level navigation is limited to the exact Tauri application origin; an exact configured development origin is additionally allowed only in debug builds;
- `window.open` requests and browser downloads are denied;
- the Content Security Policy permits ordinary renderer fetch/XHR/WebSocket/EventSource `connect-src` only for Tauri IPC (`ipc:` and `http://ipc.localhost`) and has no remote HTTP or HTTPS source;
- frames, objects, form submissions, and base-URL replacement are denied;
- PhotoForge supplies no custom `additionalBrowserArgs`, undocumented feature switch, firewall rule, or hosts-file entry.

Tauri documents that its navigation callback cancels a navigation when it returns `false`, its new-window callback can deny `window.open`, and a download callback returning `false` rejects a requested download. Tauri also recommends making the CSP as restrictive as possible and shows the IPC-only `connect-src` used here. See [Tauri's WebviewBuilder reference](https://docs.rs/tauri/2.11.5/tauri/webview/struct.WebviewBuilder.html) and [Tauri's CSP guidance](https://v2.tauri.app/security/csp/).

The optional Ollama provider does not use the renderer network path. Its bounded Rust client can contact only an explicit HTTP loopback endpoint and only after one of the documented user actions.

## Why this is not a process-tree zero-network guarantee

Microsoft states that WebView2 collects required diagnostic data regardless of the Windows optional-diagnostics setting and that an embedding application does not control overall diagnostic collection. Microsoft exposes application control for specific features such as SmartScreen, which is why PhotoForge uses the documented setting rather than a Chromium switch. See [Data and privacy in WebView2](https://learn.microsoft.com/en-us/microsoft-edge/webview2/concepts/data-privacy), [`IsReputationCheckingRequired`](https://learn.microsoft.com/en-us/dotnet/api/microsoft.web.webview2.core.corewebview2settings.isreputationcheckingrequired), and the Win32 [`ICoreWebView2Settings8`](https://learn.microsoft.com/en-us/microsoft-edge/webview2/reference/win32/icorewebview2settings8) interface used by the Rust host.

Microsoft also documents an administrator policy named `ExperimentationAndConfigurationServiceControl`. Restricted mode stops that particular configuration service, but Microsoft does not recommend the setting and does not describe it as disabling required diagnostics. It is a machine-management decision, not an application-scoped control, so PhotoForge does not write the WebView2 policy registry. See [Microsoft Edge WebView2 policies](https://learn.microsoft.com/en-us/deployedge/microsoft-edge-webview-policies#experimentationandconfigurationservicecontrol).

Consequently, PhotoForge blocks the application-controlled remote-content surfaces enumerated above, and its Rust process connects only through explicit loopback Ollama actions. These controls are not a process firewall, and PhotoForge cannot truthfully guarantee that the shared WebView2 Runtime will open no socket.

## Post-hardening observation

A final portable executable was observed idle on 2026-08-13 for 25.187 seconds using process-owned `Get-NetTCPConnection` sampling. The executable was 14,744,064 bytes with SHA-256 `bd88cde252a1277baca7d8283af6b7ae937a002725c199a2d62936ee7076b0d2`.

- The PhotoForge Rust root owned no sampled non-loopback connection.
- WebView2 151.0.4129.78 browser-host PID 21696 opened two TLS connections to `[2603:1046:c0b:4d::2]:443`, first sampled at 1,899.6/1,899.4 ms and still present at 25,062.6/25,062.5 ms.
- Windows/system connections were recorded separately and were not attributed to PhotoForge.

The executable navigated to the PhotoForge application document only after the supported SmartScreen value had been applied and read back; otherwise startup would have returned an error. The observation therefore demonstrates that disabling SmartScreen and forbidding the enumerated application-level remote-content surfaces does not disable WebView2's independent runtime traffic.
