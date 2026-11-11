//! Browser response policy for the Web documents and for bytes Cowboy serves
//! on its own origin without having authored them.

use super::*;

/// Repository files and agent images are data. If one is ever opened as a
/// document, it gets an opaque origin and no capability to run or load.
pub(super) const UNTRUSTED_BYTES_CSP: &str = "default-src 'none'; sandbox";

/// Inline-script hashes come from the document being served, so a Web release
/// needs no matching Controller release and a cached shell keeps its own policy.
///
/// - `'wasm-unsafe-eval'`: the lazy shell formatter and syntax grammars.
/// - `style-src 'unsafe-inline'`: Emotion, CodeMirror and Mermaid inject styles.
/// - `img-src` has no remote source: untrusted content chooses image URLs, and
///   loading one is a zero-click exfiltration channel (`protectedImage.ts`).
/// - `wss://`: older WebKit does not let `'self'` cover the socket.
/// - `ipc:`: the Tauri IPC transport on the native shells.
pub(super) fn app_document_csp(html: &[u8], host: Option<&str>) -> String {
    let hashes: String = inline_script_hashes(html)
        .iter()
        .map(|hash| format!(" '{hash}'"))
        .collect();
    let socket = host
        .filter(|host| {
            host.bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b".-:[]".contains(&byte))
        })
        .map(|host| format!(" wss://{host}"))
        .unwrap_or_default();
    format!(
        "default-src 'self'; script-src 'self' 'wasm-unsafe-eval'{hashes}; \
         style-src 'self' 'unsafe-inline'; img-src 'self' data: blob:; \
         font-src 'self' data:; media-src 'self' data: blob:; \
         connect-src 'self'{socket} ipc: http://ipc.localhost; \
         worker-src 'self' blob:; object-src 'none'; base-uri 'none'; \
         form-action 'self'; frame-ancestors 'none'"
    )
}

/// Every `<script>` body without `src`. Data blocks are included on purpose: the
/// editor plugin sandbox runs one as its own inline boot script, inside a srcdoc
/// frame that inherits this policy.
fn inline_script_hashes(html: &[u8]) -> Vec<String> {
    let Ok(html) = std::str::from_utf8(html) else {
        return Vec::new();
    };
    let mut hashes = Vec::new();
    let mut rest = html;
    while let Some(start) = rest.find("<script") {
        let element = &rest[start..];
        let Some(tag_end) = element.find('>') else {
            break;
        };
        let body = &element[tag_end + 1..];
        let Some(body_end) = body.find("</script") else {
            break;
        };
        if !element[..tag_end].contains(" src=") {
            let digest = Sha256::digest(&body.as_bytes()[..body_end]);
            hashes.push(format!(
                "sha256-{}",
                base64::engine::general_purpose::STANDARD.encode(digest)
            ));
        }
        rest = &body[body_end..];
    }
    hashes
}

/// The full-DOM app shell (docs/widget-apps-dom.md §3): framed only by Cowboy,
/// in a frame that is always `sandbox="allow-scripts"`. Its one inline script
/// is authorized by its hash; everything else the app runs arrives over the
/// verified port as `blob:` URLs. No network destination is allowed, WebRTC
/// included (`Connection-Allowlist: ()`), and every powerful feature is off.
pub(super) fn app_shell_headers(html: &[u8]) -> Vec<(&'static str, String)> {
    let hashes: String = inline_script_hashes(html)
        .iter()
        .map(|hash| format!(" '{hash}'"))
        .collect();
    let csp = format!(
        "default-src 'none'; script-src{hashes} blob: 'wasm-unsafe-eval'; \
         style-src 'unsafe-inline' blob:; img-src blob: data:; font-src blob: data:; \
         media-src blob: data:; connect-src 'none'; frame-src 'none'; child-src 'none'; \
         worker-src 'none'; manifest-src 'none'; form-action 'none'; base-uri 'none'; \
         frame-ancestors 'self'; webrtc 'block'"
    );
    let features = [
        "accelerometer",
        "ambient-light-sensor",
        "autoplay",
        "bluetooth",
        "browsing-topics",
        "camera",
        "clipboard-read",
        "clipboard-write",
        "compute-pressure",
        "display-capture",
        "encrypted-media",
        "fullscreen",
        "gamepad",
        "geolocation",
        "gyroscope",
        "hid",
        "identity-credentials-get",
        "idle-detection",
        "local-fonts",
        "magnetometer",
        "microphone",
        "midi",
        "otp-credentials",
        "payment",
        "picture-in-picture",
        "publickey-credentials-create",
        "publickey-credentials-get",
        "screen-wake-lock",
        "serial",
        "speaker-selection",
        "storage-access",
        "usb",
        "web-share",
        "window-management",
        "xr-spatial-tracking",
    ];
    let permissions = features
        .iter()
        .map(|feature| format!("{feature}=()"))
        .collect::<Vec<_>>()
        .join(", ");
    vec![
        ("content-type", "text/html; charset=utf-8".to_owned()),
        ("cache-control", "no-store".to_owned()),
        ("content-security-policy", csp),
        ("connection-allowlist", "()".to_owned()),
        ("permissions-policy", permissions),
        ("referrer-policy", "no-referrer".to_owned()),
        ("x-content-type-options", "nosniff".to_owned()),
        ("cross-origin-resource-policy", "same-origin".to_owned()),
    ]
}

pub(super) fn harden_static_response(headers: &mut HeaderMap, document_csp: Option<&str>) {
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        header::HeaderValue::from_static("nosniff"),
    );
    let Some(csp) = document_csp.and_then(|csp| header::HeaderValue::from_str(csp).ok()) else {
        return;
    };
    headers.insert(header::CONTENT_SECURITY_POLICY, csp);
    headers.insert(
        header::X_FRAME_OPTIONS,
        header::HeaderValue::from_static("DENY"),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn digest(body: &str) -> String {
        format!(
            "'sha256-{}'",
            base64::engine::general_purpose::STANDARD.encode(Sha256::digest(body.as_bytes()))
        )
    }

    #[test]
    fn the_policy_hashes_the_inline_scripts_of_the_served_document() {
        let html = concat!(
            "<!doctype html><script>a()</script>\n",
            r#"<script type="module" crossorigin src="/assets/x.js"></script>"#,
            r#"<script>const tag = /<script\b[^>]*>/;</script>"#,
            r#"<script type="text/plain" id="boot">boot()</script>"#,
        );
        let csp = app_document_csp(html.as_bytes(), Some("cowboy.example"));
        assert!(csp.contains(&digest("a()")));
        assert!(csp.contains(&digest(r"const tag = /<script\b[^>]*>/;")));
        assert!(csp.contains(&digest("boot()")));
        assert!(!csp.contains(&digest("")));
        let script_src = csp
            .split(';')
            .find(|directive| directive.trim().starts_with("script-src"))
            .unwrap();
        assert!(!script_src.contains("unsafe-inline"));
        assert!(!script_src.contains("'unsafe-eval'"));
        assert!(csp.contains("connect-src 'self' wss://cowboy.example ipc:"));
        assert!(csp.contains("frame-ancestors 'none'"));
    }

    #[test]
    fn an_implausible_host_never_reaches_the_policy() {
        let csp = app_document_csp(b"", Some("evil; script-src *"));
        assert!(csp.contains("connect-src 'self' ipc:"));
        assert!(!csp.contains("evil"));
    }

    #[test]
    fn the_app_shell_denies_every_destination_and_hashes_its_script() {
        let html = b"<!doctype html><div id=root></div><script>boot()</script>";
        let headers: std::collections::HashMap<_, _> =
            app_shell_headers(html).into_iter().collect();
        let csp = &headers["content-security-policy"];
        assert!(csp.contains(&digest("boot()")));
        for directive in [
            "default-src 'none'",
            "connect-src 'none'",
            "frame-src 'none'",
            "worker-src 'none'",
            "frame-ancestors 'self'",
        ] {
            assert!(csp.contains(directive), "{directive}");
        }
        assert!(
            !csp.contains("'self' "),
            "no network-loaded script or style: {csp}"
        );
        assert_eq!(headers["connection-allowlist"], "()");
        assert!(headers["permissions-policy"].contains("camera=()"));
        assert!(!headers.contains_key("x-frame-options"));
        assert!(!headers.contains_key("set-cookie"));
    }

    #[test]
    fn only_documents_carry_the_document_policy() {
        let mut headers = HeaderMap::new();
        harden_static_response(&mut headers, None);
        assert_eq!(headers[header::X_CONTENT_TYPE_OPTIONS], "nosniff");
        assert!(!headers.contains_key(header::CONTENT_SECURITY_POLICY));
        harden_static_response(&mut headers, Some("default-src 'self'"));
        assert_eq!(headers[header::X_FRAME_OPTIONS], "DENY");
    }
}
