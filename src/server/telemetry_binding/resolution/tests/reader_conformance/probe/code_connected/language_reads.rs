//! Legacy queries borrow an already open native buffer. Their HTTP authority
//! must not survive logout or acquire/release resources as a side effect of
//! refusing the reply. Native query-internal effects are not compensated here.
use super::*;

pub(super) const FILE: &str = "fixture.txt";
pub(super) const COMMANDS: [&str; 4] = [
    "bufferLanguage",
    "bufferHover",
    "bufferNavigate",
    "bufferSymbols",
];

pub(super) async fn authorization(
    pair: &mut Pair<'_>,
    password: &str,
    stage: &mut &'static str,
    checks: &mut Vec<&'static str>,
) -> Result<(), Failure> {
    for (route, command, field, name) in [
        (
            "language",
            COMMANDS[0],
            "version",
            "legacy_language_original_login_revocation",
        ),
        (
            "intelligence/hover",
            COMMANDS[1],
            "contents",
            "legacy_hover_original_login_revocation",
        ),
        (
            "intelligence/navigation",
            COMMANDS[2],
            "locations",
            "legacy_navigation_original_login_revocation",
        ),
        (
            "intelligence/outline",
            COMMANDS[3],
            "symbols",
            "legacy_outline_original_login_revocation",
        ),
    ] {
        *stage = name;
        let path = format!(
            "/api/code/sessions/{SESSION}/{route}?path={FILE}&row=0&column=3&kind=definition"
        );
        let before = pair.proxy.counts()?.commands;
        let first = pair.http.get(&path).await?;
        check(first["apiVersion"] == 1 && first["path"] == FILE && first[field].is_array())?;
        let mut reader = Http::new(pair.address)?;
        reader.login(password).await?;
        // Preserve the failing HTTP client on Pair for exact negative evidence.
        let original = std::mem::replace(&mut pair.http, reader);
        read_routes::revoked_read(pair, &path, command).await?;
        pair.http = original;
        let fresh = pair.http.get(&path).await?;
        check(fresh["apiVersion"] == 1 && fresh["path"] == FILE && fresh[field].is_array())?;
        let mut expected = before;
        *expected.entry(command.into()).or_default() += 3;
        // No implicit reopen, release, replay or other native command.
        check(pair.proxy.counts()?.commands == expected)?;
        checks.push(name);
    }
    Ok(())
}
