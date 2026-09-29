//! Machine-local host ports and generic, bounded resource projection.

use std::collections::BTreeMap;
use std::path::Path;

use cowboy_plugin_sdk::{WorkspaceResourceFields, WorkspaceResourceView};
use serde_json::Value;

use super::{Failure, Metadata, Remote, Resource, Response};

const MAX_BODY: usize = 64 * 1024;

fn segment(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 100
        && value != "."
        && value != ".."
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
}

pub(super) fn normalize_remote(name: &str, raw: &str) -> Option<Remote> {
    if !segment(name) {
        return None;
    }
    let normalized = if let Some(path) = raw.strip_prefix("git@") {
        let (host, path) = path.split_once(':')?;
        format!("ssh://git@{host}/{path}")
    } else {
        raw.to_owned()
    };
    let url = url::Url::parse(&normalized).ok()?;
    if !matches!(url.scheme(), "https" | "ssh")
        || url.password().is_some()
        || url.port().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return None;
    }
    if url.scheme() == "https" && !url.username().is_empty() {
        return None;
    }
    let host = url.host_str()?.to_ascii_lowercase();
    if host.len() > 253
        || !host.contains('.')
        || !host
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b".-".contains(&b))
    {
        return None;
    }
    let path = url
        .path()
        .trim_matches('/')
        .strip_suffix(".git")
        .unwrap_or(url.path().trim_matches('/'));
    let (owner, repo) = path.split_once('/')?;
    if !segment(owner) || !segment(repo) {
        return None;
    }
    Some(Remote {
        name: name.to_owned(),
        host,
        owner: owner.to_owned(),
        repository: repo.to_owned(),
    })
}

fn environment() -> BTreeMap<String, String> {
    // This explicit port borrows the Machine owner's existing CLI session.
    // Provider homes, agent tokens and debug switches are never inherited.
    let mut values = BTreeMap::new();
    for key in [
        "HOME",
        "PATH",
        "XDG_CONFIG_HOME",
        "GH_CONFIG_DIR",
        "XDG_RUNTIME_DIR",
        "DBUS_SESSION_BUS_ADDRESS",
        "SSL_CERT_FILE",
        "SSL_CERT_DIR",
        "HTTPS_PROXY",
        "HTTP_PROXY",
        "NO_PROXY",
    ] {
        if let Ok(value) = std::env::var(key) {
            values.insert(key.to_owned(), value);
        }
    }
    for (key, value) in [
        ("GH_PROMPT_DISABLED", "1"),
        ("GH_PAGER", "cat"),
        ("PAGER", "cat"),
        ("GH_NO_UPDATE_NOTIFIER", "1"),
        ("GH_NO_EXTENSION_UPDATE_NOTIFIER", "1"),
        ("GIT_TERMINAL_PROMPT", "0"),
    ] {
        values.insert(key.to_owned(), value.to_owned());
    }
    values
}

async fn command(program: &str, args: &[String]) -> Result<Vec<u8>, Failure> {
    let output = crate::plugin_process::run_plugin_command_with_environment(
        program,
        args,
        &[],
        &environment(),
    )
    .await
    .map_err(|_| Failure::ConnectionUnavailable)?;
    if !output.status.success() {
        return Err(Failure::RequestFailed);
    }
    Ok(output.stdout)
}

pub(crate) async fn remotes(root: &Path) -> Result<Vec<Remote>, Failure> {
    let args = [
        "-C",
        root.to_str().ok_or(Failure::RepositoryUnavailable)?,
        "config",
        "--get-regexp",
        "^remote\\..*\\.url$",
    ]
    .map(str::to_owned);
    let bytes = command("git", &args)
        .await
        .map_err(|_| Failure::RepositoryUnavailable)?;
    let source = std::str::from_utf8(&bytes).map_err(|_| Failure::RepositoryUnavailable)?;
    let mut result = BTreeMap::new();
    for line in source.lines().take(64) {
        let Some((key, value)) = line.split_once(' ') else {
            continue;
        };
        let Some(name) = key
            .strip_prefix("remote.")
            .and_then(|s| s.strip_suffix(".url"))
        else {
            continue;
        };
        if let Some(remote) = normalize_remote(name, value.trim()) {
            result.insert(name.to_owned(), remote);
        }
    }
    let mut result: Vec<_> = result.into_values().collect();
    result.sort_by_key(|r| (r.name != "origin", r.name.clone()));
    Ok(result)
}

fn value(source: &Value, pointer: &str, max: usize) -> Option<String> {
    let value = source.pointer(pointer)?;
    let value = match value {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        Value::Bool(b) => b.to_string(),
        _ => return None,
    };
    if value.len() > max
        || value
            .chars()
            .any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t'))
    {
        return None;
    }
    Some(value)
}

fn project(
    source: &Value,
    fields: &WorkspaceResourceFields,
    remote: &Remote,
    detail: bool,
) -> Option<Resource> {
    let id = value(source, &fields.id, 24)?;
    if id.is_empty() || !id.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let title = value(source, &fields.title, 1000)?;
    let url = value(source, &fields.url, 2048).filter(|s| {
        url::Url::parse(s).is_ok_and(|u| {
            u.scheme() == "https"
                && u.host_str() == Some(remote.host.as_str())
                && u.username().is_empty()
                && u.password().is_none()
                && u.port().is_none()
                && u.path()
                    .starts_with(&format!("/{}/{}/", remote.owner, remote.repository))
        })
    });
    let mut body = detail
        .then(|| {
            fields
                .body
                .as_ref()
                .and_then(|p| value(source, p, 1024 * 1024))
        })
        .flatten();
    let body_truncated = body.as_ref().is_some_and(|body| body.len() > MAX_BODY);
    if let Some(body) = body.as_mut().filter(|body| body.len() > MAX_BODY) {
        let mut end = MAX_BODY;
        while !body.is_char_boundary(end) {
            end -= 1;
        }
        body.truncate(end);
    }
    Some(Resource {
        id,
        title,
        url,
        body,
        body_truncated,
        state: fields.state.as_ref().and_then(|p| value(source, p, 80)),
        updated_at: fields
            .updated_at
            .as_ref()
            .and_then(|p| value(source, p, 64)),
        metadata: fields
            .metadata
            .iter()
            .filter_map(|m| {
                Some(Metadata {
                    label: m.label.clone(),
                    value: value(source, &m.pointer, 256)?,
                })
            })
            .collect(),
    })
}

pub(crate) async fn read(
    remote: &Remote,
    view: &WorkspaceResourceView,
    item: Option<&str>,
    filter: Option<&str>,
    page: u32,
) -> Result<Response, Failure> {
    read_with_cli("gh", remote, view, item, filter, page).await
}

async fn read_with_cli(
    program: &str,
    remote: &Remote,
    view: &WorkspaceResourceView,
    item: Option<&str>,
    filter: Option<&str>,
    page: u32,
) -> Result<Response, Failure> {
    if !(1..=1000).contains(&page)
        || item.is_some_and(|id| {
            id.is_empty() || id.len() > 24 || !id.bytes().all(|b| b.is_ascii_digit())
        })
    {
        return Err(Failure::InvalidRequest);
    }
    let filter = filter
        .or_else(|| view.filters.first().map(|f| f.value.as_str()))
        .unwrap_or("");
    if (view.filters.is_empty() && !filter.is_empty())
        || (!view.filters.is_empty() && !view.filters.iter().any(|f| f.value == filter))
    {
        return Err(Failure::InvalidRequest);
    }
    // auth status never exports a token; do not pass --show-token or read files.
    command(
        program,
        &["auth", "status", "--active", "--hostname", &remote.host].map(str::to_owned),
    )
    .await
    .map_err(|_| Failure::ConnectionUnavailable)?;
    let endpoint = if item.is_some() {
        &view.detail_endpoint
    } else {
        &view.endpoint
    }
    .replace("{owner}", &remote.owner)
    .replace("{repo}", &remote.repository)
    .replace("{id}", item.unwrap_or(""))
    .replace("{page}", &page.to_string())
    .replace("{filter}", filter);
    let bytes = command(
        program,
        &[
            "api",
            "--method",
            "GET",
            "--hostname",
            &remote.host,
            "--header",
            "Accept: application/vnd.github+json",
            &endpoint,
        ]
        .map(str::to_owned),
    )
    .await?;
    let data: Value = serde_json::from_slice(&bytes).map_err(|_| Failure::RequestFailed)?;
    if let Some(requested) = item {
        let projected = project(&data, &view.fields, remote, true).ok_or(Failure::RequestFailed)?;
        if projected.id != requested {
            return Err(Failure::RequestFailed);
        }
        return Ok(Response::Detail { item: projected });
    }
    let rows = data
        .pointer(&view.items_pointer)
        .and_then(Value::as_array)
        .ok_or(Failure::RequestFailed)?;
    if rows.len() > 50 {
        return Err(Failure::RequestFailed);
    }
    let items = rows
        .iter()
        .filter(|r| {
            view.exclude_if_present
                .as_ref()
                .is_none_or(|p| r.pointer(p).is_none())
        })
        .filter_map(|r| project(r, &view.fields, remote, false))
        .collect();
    Ok(Response::Page {
        items,
        next_page: (rows.len() == 50 && page < 1000).then_some(page + 1),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt as _;

    #[test]
    fn remote_identity_is_confined_and_never_exports_credentials() {
        let ssh = normalize_remote("origin", "git@github.com:owner/repo.git").unwrap();
        assert_eq!(
            ssh,
            normalize_remote("origin", "https://github.com/owner/repo.git").unwrap()
        );
        for raw in [
            "https://token@github.com/owner/repo",
            "https://github.com/owner/repo/extra",
            "file:///tmp/repo",
            "https://github.com/owner/../repo",
            "https://127.0.0.1:3333/owner/repo",
            "https://github.com/owner/%2e%2e",
            "git@github.com:owner/repo?token=x",
        ] {
            assert!(normalize_remote("origin", raw).is_none(), "{raw}");
        }
    }

    #[tokio::test]
    async fn cli_reads_are_bounded_gets_and_only_project_declared_resource_fields() {
        let root =
            std::env::temp_dir().join(format!("cowboy-extension-cli-{}", rand::random::<u64>()));
        std::fs::create_dir(&root).unwrap();
        let executable = root.join("gh-fixture");
        let data = root.join("data.json");
        // No credentials: the script only verifies argv/environment and serves fixture bytes.
        std::fs::write(&executable, format!(r#"#!/bin/sh
test -z "$GH_TOKEN$GITHUB_TOKEN$GH_DEBUG" || exit 42
test "$GH_PROMPT_DISABLED" = 1 || exit 43
if [ "$1" = auth ]; then
  test "$2:$3:$4:$5" = 'status:--active:--hostname:github.com' || exit 44
  exit 0
fi
test "$1:$2:$3:$4:$5:$6:$7" = 'api:--method:GET:--hostname:github.com:--header:Accept: application/vnd.github+json' || exit 45
case "$8" in 'repos/owner/repo/issues?state=open&per_page=50&page=1'|'repos/owner/repo/issues/12') ;; *) exit 46 ;; esac
cat '{}'
"#, data.display())).unwrap();
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
        let contract: cowboy_plugin_sdk::WorkspaceExtensionContract =
            serde_json::from_str(include_str!("../../plugins/github/contract.json")).unwrap();
        let view = contract.views.iter().find(|v| v.id == "issues").unwrap();
        let remote = normalize_remote("origin", "git@github.com:owner/repo.git").unwrap();
        let row = serde_json::json!({ "number":12, "title":"Resource", "html_url":"https://github.com/owner/repo/issues/12", "body":"Description", "state":"open", "secret":"not-projected" });
        std::fs::write(
            &data,
            serde_json::to_vec(
                &serde_json::json!([row.clone(), { "pull_request":{}, "number":13 }]),
            )
            .unwrap(),
        )
        .unwrap();
        let result = read_with_cli(
            executable.to_str().unwrap(),
            &remote,
            view,
            None,
            Some("open"),
            1,
        )
        .await
        .unwrap();
        let encoded = serde_json::to_string(&result).unwrap();
        assert!(!encoded.contains("not-projected"));
        let Response::Page { items, next_page } = result else {
            panic!("expected collection");
        };
        assert_eq!(items.len(), 1);
        assert!(items[0].body.is_none());
        assert_eq!(next_page, None);
        std::fs::write(&data, serde_json::to_vec(&row).unwrap()).unwrap();
        let Response::Detail { item } = read_with_cli(
            executable.to_str().unwrap(),
            &remote,
            view,
            Some("12"),
            None,
            1,
        )
        .await
        .unwrap() else {
            panic!("expected detail");
        };
        assert_eq!(item.body.as_deref(), Some("Description"));
        std::fs::write(&data, serde_json::to_vec(&vec![row; 51]).unwrap()).unwrap();
        assert!(matches!(
            read_with_cli(executable.to_str().unwrap(), &remote, view, None, None, 1).await,
            Err(Failure::RequestFailed)
        ));
        assert!(matches!(
            read_with_cli(
                executable.to_str().unwrap(),
                &remote,
                view,
                Some("../secret"),
                None,
                1
            )
            .await,
            Err(Failure::InvalidRequest)
        ));
        std::fs::write(
            &executable,
            "#!/bin/sh\necho 'secret raw error' >&2\nexit 1\n",
        )
        .unwrap();
        assert!(matches!(
            read_with_cli(executable.to_str().unwrap(), &remote, view, None, None, 1).await,
            Err(Failure::ConnectionUnavailable)
        ));
        std::fs::remove_dir_all(root).unwrap();
    }
}
