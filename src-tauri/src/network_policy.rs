use std::{error::Error, fmt, sync::Arc};

use tauri::{
    utils::config::WebviewUrl,
    webview::{DownloadEvent, NewWindowResponse},
    Url, WebviewWindowBuilder,
};

#[derive(Clone, Debug, Eq, PartialEq)]
struct Origin {
    scheme: String,
    host: String,
    port: Option<u16>,
}

impl Origin {
    fn from_url(url: &Url) -> Option<Self> {
        if !url.username().is_empty() || url.password().is_some() {
            return None;
        }

        Some(Self {
            scheme: url.scheme().to_owned(),
            host: url.host_str()?.to_owned(),
            port: url.port(),
        })
    }
}

#[derive(Clone, Debug)]
struct NavigationPolicy {
    bundled_origin: Origin,
    development_origin: Option<Origin>,
}

impl NavigationPolicy {
    fn new(use_https_scheme: bool, development_url: Option<&Url>) -> Self {
        let bundled_url = Url::parse(if use_https_scheme {
            "https://tauri.localhost"
        } else {
            "http://tauri.localhost"
        })
        .expect("the fixed Tauri application origin must be valid");

        Self {
            bundled_origin: Origin::from_url(&bundled_url)
                .expect("the fixed Tauri application origin must have a host"),
            development_origin: development_url.and_then(Origin::from_url),
        }
    }

    fn allows(&self, url: &Url) -> bool {
        let Some(origin) = Origin::from_url(url) else {
            return false;
        };

        origin == self.bundled_origin || self.development_origin.as_ref() == Some(&origin)
    }
}

#[derive(Debug)]
struct NetworkPolicyError(String);

impl fmt::Display for NetworkPolicyError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for NetworkPolicyError {}

pub fn create_main_window(app: &tauri::App) -> Result<(), Box<dyn Error>> {
    let mut window_config = app
        .config()
        .app
        .windows
        .iter()
        .find(|window| window.label == "main")
        .cloned()
        .ok_or_else(|| NetworkPolicyError("the main window configuration is missing".into()))?;

    let development_url = if cfg!(debug_assertions) {
        app.config().build.dev_url.as_ref()
    } else {
        None
    };
    let navigation_policy = NavigationPolicy::new(window_config.use_https_scheme, development_url);
    let application_url = resolve_application_url(
        &window_config.url,
        window_config.use_https_scheme,
        development_url,
    )?;

    // Create WebView2 on an inert asset at the normal Tauri application origin
    // so Tauri's per-webview origin metadata remains correct. The supported
    // privacy setting is committed before navigation to the actual application
    // document. Microsoft documents that a changed reputation-checking setting
    // applies on the next navigation.
    window_config.url = WebviewUrl::App("bootstrap.html".into());

    let window = WebviewWindowBuilder::from_config(app, &window_config)?
        .on_navigation(move |url| navigation_policy.allows(url))
        .on_new_window(|_, _| NewWindowResponse::Deny)
        .on_download(|_, event| !matches!(event, DownloadEvent::Requested { .. }))
        .build()?;

    configure_platform_privacy(&window)?;
    window.navigate(application_url)?;
    Ok(())
}

fn resolve_application_url(
    configured_url: &WebviewUrl,
    use_https_scheme: bool,
    development_url: Option<&Url>,
) -> Result<Url, Box<dyn Error>> {
    match configured_url {
        WebviewUrl::External(url) | WebviewUrl::CustomProtocol(url) => Ok(url.clone()),
        WebviewUrl::App(path) => {
            let mut base = if let Some(development_url) = development_url {
                development_url.clone()
            } else {
                Url::parse(if use_https_scheme {
                    "https://tauri.localhost"
                } else {
                    "http://tauri.localhost"
                })?
            };

            if path.to_str() != Some("index.html") {
                base = base.join(&path.to_string_lossy())?;
            }
            Ok(base)
        }
        _ => Err(Box::new(NetworkPolicyError(
            "the configured main window URL type is unsupported".into(),
        ))),
    }
}

#[cfg(windows)]
fn configure_platform_privacy(window: &tauri::WebviewWindow) -> Result<(), Box<dyn Error>> {
    use std::sync::Mutex;

    use webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2Settings8;
    use windows_core::{Interface, BOOL};

    let outcome = Arc::new(Mutex::new(None));
    let callback_outcome = Arc::clone(&outcome);

    window.with_webview(move |platform_webview| {
        let result = (|| -> Result<(), String> {
            let controller = platform_webview.controller();
            let webview =
                unsafe { controller.CoreWebView2() }.map_err(|error| error.to_string())?;
            let settings = unsafe { webview.Settings() }.map_err(|error| error.to_string())?;
            let settings: ICoreWebView2Settings8 =
                settings.cast().map_err(|error| error.to_string())?;

            unsafe { settings.SetIsReputationCheckingRequired(false) }
                .map_err(|error| error.to_string())?;

            let mut reputation_checking_required = BOOL::default();
            unsafe { settings.IsReputationCheckingRequired(&mut reputation_checking_required) }
                .map_err(|error| error.to_string())?;
            if reputation_checking_required.0 != 0 {
                return Err("WebView2 did not disable reputation checking".into());
            }

            Ok(())
        })();

        *callback_outcome
            .lock()
            .expect("the WebView2 privacy result mutex was poisoned") = Some(result);
    })?;

    // This call originates from Tauri's setup callback on the main thread. The
    // pinned Tauri 2.11 runtime executes `with_webview` inline on that thread.
    let result = outcome
        .lock()
        .map_err(|_| NetworkPolicyError("the WebView2 privacy result mutex was poisoned".into()))?
        .take()
        .ok_or_else(|| {
            NetworkPolicyError("the WebView2 privacy callback did not execute during setup".into())
        })?;

    result.map_err(|message| {
        Box::new(NetworkPolicyError(format!(
            "failed to apply the WebView2 privacy policy: {message}"
        ))) as Box<dyn Error>
    })
}

#[cfg(not(windows))]
fn configure_platform_privacy(_: &tauri::WebviewWindow) -> Result<(), Box<dyn Error>> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn url(value: &str) -> Url {
        Url::parse(value).unwrap()
    }

    #[test]
    fn bundled_navigation_accepts_only_the_exact_tauri_origin() {
        let policy = NavigationPolicy::new(false, None);

        assert!(policy.allows(&url("http://tauri.localhost/")));
        assert!(policy.allows(&url("http://tauri.localhost/editor?tab=mask#active")));
        // URL parsing normalizes an explicit default port to the same origin.
        assert!(policy.allows(&url("http://tauri.localhost:80/")));
        assert!(!policy.allows(&url("https://tauri.localhost/")));
        assert!(!policy.allows(&url("http://tauri.localhost.example/")));
        assert!(!policy.allows(&url("http://user@tauri.localhost/")));
        assert!(!policy.allows(&url("https://example.com/")));
        assert!(!policy.allows(&url("file:///C:/photo.jpg")));
        assert!(!policy.allows(&url("data:text/html,hello")));
    }

    #[test]
    fn https_bundled_origin_is_explicit() {
        let policy = NavigationPolicy::new(true, None);

        assert!(policy.allows(&url("https://tauri.localhost/")));
        assert!(!policy.allows(&url("http://tauri.localhost/")));
    }

    #[test]
    fn development_navigation_accepts_only_the_configured_origin() {
        let development_url = url("http://127.0.0.1:1420/");
        let policy = NavigationPolicy::new(false, Some(&development_url));

        assert!(policy.allows(&url("http://127.0.0.1:1420/src/main.ts")));
        assert!(!policy.allows(&url("http://localhost:1420/")));
        assert!(!policy.allows(&url("http://127.0.0.1:1421/")));
        assert!(!policy.allows(&url("https://127.0.0.1:1420/")));
    }

    #[test]
    fn release_policy_does_not_allow_the_configured_dev_server() {
        let policy = NavigationPolicy::new(false, None);

        assert!(!policy.allows(&url("http://127.0.0.1:1420/")));
    }

    #[test]
    fn application_url_resolves_after_the_privacy_bootstrap() {
        let index = WebviewUrl::App("index.html".into());
        let nested = WebviewUrl::App("editor/index.html".into());
        let development_url = url("http://127.0.0.1:1420/");

        assert_eq!(
            resolve_application_url(&index, false, None)
                .unwrap()
                .as_str(),
            "http://tauri.localhost/"
        );
        assert_eq!(
            resolve_application_url(&index, true, None)
                .unwrap()
                .as_str(),
            "https://tauri.localhost/"
        );
        assert_eq!(
            resolve_application_url(&nested, false, Some(&development_url))
                .unwrap()
                .as_str(),
            "http://127.0.0.1:1420/editor/index.html"
        );
    }

    #[test]
    fn tauri_config_keeps_renderer_connections_and_window_creation_fail_closed() {
        let config: serde_json::Value =
            serde_json::from_str(include_str!("../tauri.conf.json")).unwrap();
        let main_window = &config["app"]["windows"][0];
        let csp = config["app"]["security"]["csp"].as_str().unwrap();

        assert_eq!(main_window["label"], "main");
        assert_eq!(main_window["create"], false);
        assert!(main_window.get("additionalBrowserArgs").is_none());
        assert!(csp.contains("connect-src ipc: http://ipc.localhost"));
        assert!(!csp.contains("https:"));
        assert!(!csp.contains("http: "));
        assert!(csp.contains("object-src 'none'"));
        assert!(csp.contains("frame-src 'none'"));
        assert!(csp.contains("form-action 'none'"));
    }

    #[test]
    fn bundled_bootstrap_is_inert_and_script_free() {
        let bootstrap = include_str!("../../public/bootstrap.html");

        assert!(bootstrap.contains("Content-Security-Policy"));
        assert!(bootstrap.contains("default-src 'none'"));
        assert!(!bootstrap.contains("<script"));
        assert!(!bootstrap.contains("http://"));
        assert!(!bootstrap.contains("https://"));
    }
}
