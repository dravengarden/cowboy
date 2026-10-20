use tauri::{webview::NewWindowResponse, AppHandle, Manager, Url, WebviewWindowBuilder, Wry};
use tauri_plugin_opener::OpenerExt;

fn trusted_document(url: &Url) -> bool {
    (url.scheme() == "https"
        && url.host_str() == Some("cowboy.stormbird.xyz")
        && url.port_or_known_default() == Some(443))
        || (url.scheme() == "tauri" && url.host_str() == Some("localhost"))
        || (matches!(url.scheme(), "http" | "https")
            && url.host_str() == Some("tauri.localhost")
            && url.port().is_none())
}

fn external_url(url: &Url) -> bool {
    matches!(url.scheme(), "http" | "https" | "mailto" | "tel") && !trusted_document(url)
}

fn open_from_main(app: &AppHandle, url: &Url) {
    // Native menu/window requests bypass the page's click handlers and IPC ACL.
    // Keep the same source-origin and protocol boundary as the remote capability.
    let trusted = app
        .get_webview_window("main")
        .and_then(|window| window.url().ok())
        .is_some_and(|source| trusted_document(&source));
    if trusted && matches!(url.scheme(), "http" | "https" | "mailto" | "tel") {
        // Never log the URL: Provider authorization links may contain secrets.
        if app.opener().open_url(url.as_str(), None::<&str>).is_err() {
            eprintln!("Cowboy could not open the external link in the system browser");
        }
    }
}

pub fn configure(
    window: WebviewWindowBuilder<'_, Wry, tauri::App>,
    app: AppHandle,
) -> WebviewWindowBuilder<'_, Wry, tauri::App> {
    let navigation = app.clone();
    window
        .on_new_window(move |url, _| {
            open_from_main(&app, &url);
            NewWindowResponse::Deny
        })
        .on_navigation(move |url| {
            if external_url(url) {
                open_from_main(&navigation, url);
                false
            } else {
                trusted_document(url)
            }
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn links_preserve_service_navigation_and_restrict_browser_handoffs() {
        for address in [
            "https://cowboy.stormbird.xyz/",
            "tauri://localhost/index.html",
            "http://tauri.localhost/index.html",
        ] {
            let url = Url::parse(address).unwrap();
            assert!(trusted_document(&url));
            assert!(!external_url(&url));
        }
        for address in [
            "https://suger.awsapps.com/start/#/device",
            "http://localhost:8080",
            "mailto:a@example.com",
            "tel:+123",
        ] {
            assert!(external_url(&Url::parse(address).unwrap()));
        }
        for address in [
            "file:///etc/passwd",
            "javascript:alert(1)",
            "app-settings:",
            "tauri://foreign/index.html",
        ] {
            let url = Url::parse(address).unwrap();
            assert!(!trusted_document(&url));
            assert!(!external_url(&url));
        }
        assert!(!trusted_document(
            &Url::parse("https://cowboy.stormbird.xyz:444/").unwrap()
        ));
        assert!(!trusted_document(
            &Url::parse("https://cowboy.stormbird.xyz.attacker.example/").unwrap()
        ));
    }
}
