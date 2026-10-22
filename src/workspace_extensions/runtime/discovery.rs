//! Account-scoped PR discovery through fixed, bounded GitHub GET requests.
use serde_json::Value;

use super::{review::json, segment, value};
use crate::workspace_extensions::{Failure, PullDiscovery, PullSummary, Remote, Response};

const PAGE_SIZE: u64 = 20;
/// Complete pull-request objects are far larger than search rows; a smaller
/// page keeps one response inside the command's output bound.
const BRANCH_PAGE_SIZE: usize = 10;
const BRANCH_PAGES: u32 = 10;

pub fn review_target(remote: &Remote, repository: Option<&str>) -> Result<Remote, Failure> {
    let Some(repository) = repository else {
        return Ok(remote.clone());
    };
    let (owner, repo) = repository.split_once('/').ok_or(Failure::InvalidRequest)?;
    if !segment(owner) || !segment(repo) {
        return Err(Failure::InvalidRequest);
    }
    // The workspace remote pins the CLI host. Cross-repository selection never
    // supplies a hostname, API endpoint or credential to the command port.
    Ok(Remote {
        owner: owner.into(),
        repository: repo.into(),
        ..remote.clone()
    })
}

fn query(
    remote: &Remote,
    filters: &PullDiscovery,
    account: &str,
    page: u32,
) -> Result<String, Failure> {
    if !(1..=50).contains(&page) || !segment(account) {
        return Err(Failure::InvalidRequest);
    }
    let mut search = "is:pr".to_owned();
    match filters.relation.as_str() {
        "author" => search.push_str(&format!(" author:{account}")),
        "review" => search.push_str(&format!(" review-requested:{account}")),
        "assigned" => search.push_str(&format!(" assignee:{account}")),
        "all" if filters.current_repository => {}
        // Avoid an accidental global public-PR firehose.
        "all" => search.push_str(&format!(" involves:{account}")),
        _ => return Err(Failure::InvalidRequest),
    }
    match filters.state.as_str() {
        "open" => search.push_str(" is:open"),
        "closed" => search.push_str(" is:closed is:unmerged"),
        "merged" => search.push_str(" is:merged"),
        "all" => {}
        _ => return Err(Failure::InvalidRequest),
    }
    if filters.current_repository {
        search.push_str(&format!(" repo:{}/{}", remote.owner, remote.repository));
    }
    let params = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("q", &search)
        .append_pair("sort", "updated")
        .append_pair("order", "desc")
        .append_pair("per_page", "20")
        .append_pair("page", &page.to_string())
        .finish();
    Ok(format!("search/issues?{params}"))
}

/// The repository's open pull requests with the branches they join, so a
/// worktree can be matched to its pull request without a per-branch request.
fn branches_endpoint(
    remote: &Remote,
    filters: &PullDiscovery,
    page: u32,
) -> Result<String, Failure> {
    if !(1..=BRANCH_PAGES).contains(&page) || filters.state != "open" || !filters.current_repository
    {
        return Err(Failure::InvalidRequest);
    }
    Ok(format!(
        "repos/{}/{}/pulls?state=open&sort=updated&direction=desc&per_page={BRANCH_PAGE_SIZE}&page={page}",
        remote.owner, remote.repository
    ))
}

fn branch_summary(source: &Value, remote: &Remote) -> Result<PullSummary, Failure> {
    // The list endpoint has no search-style `pull_request` member; supply the
    // marker so both shapes pass the one URL and state projection.
    let mut row = source.clone();
    let merged_at = source.get("merged_at").cloned().unwrap_or(Value::Null);
    row.as_object_mut().ok_or(Failure::RequestFailed)?.insert(
        "pull_request".into(),
        serde_json::json!({ "merged_at": merged_at }),
    );
    let mut item = summary(&row, remote)?;
    // Git reference names are not path segments: bound them, nothing more.
    let reference = |pointer| value(source, pointer, 255).filter(|name| !name.is_empty());
    item.head_branch = Some(reference("/head/ref").ok_or(Failure::RequestFailed)?);
    item.base_branch = Some(reference("/base/ref").ok_or(Failure::RequestFailed)?);
    item.head_repository = value(source, "/head/repo/full_name", 201).filter(|name| {
        name.split_once('/')
            .is_some_and(|(owner, repository)| segment(owner) && segment(repository))
    });
    Ok(item)
}

fn summary(source: &Value, remote: &Remote) -> Result<PullSummary, Failure> {
    let number = value(source, "/number", 16)
        .filter(|s| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()))
        .ok_or(Failure::RequestFailed)?;
    let url = value(source, "/html_url", 2048).ok_or(Failure::RequestFailed)?;
    let parsed = url::Url::parse(&url).map_err(|_| Failure::RequestFailed)?;
    let parts: Vec<_> = parsed.path().trim_start_matches('/').split('/').collect();
    if parsed.scheme() != "https"
        || parsed.host_str() != Some(&remote.host)
        || parsed.port().is_some()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.query().is_some()
        || parsed.fragment().is_some()
        || parts.len() != 4
        || parts[2] != "pull"
        || parts[3] != number
        || !segment(parts[0])
        || !segment(parts[1])
        || !source["pull_request"].is_object()
    {
        return Err(Failure::RequestFailed);
    }
    let state = if source
        .pointer("/pull_request/merged_at")
        .is_some_and(|v| !v.is_null())
    {
        "merged".into()
    } else {
        value(source, "/state", 16)
            .filter(|s| matches!(s.as_str(), "open" | "closed"))
            .ok_or(Failure::RequestFailed)?
    };
    Ok(PullSummary {
        repository: format!("{}/{}", parts[0], parts[1]),
        number,
        title: value(source, "/title", 1000).ok_or(Failure::RequestFailed)?,
        url,
        author: value(source, "/user/login", 100).ok_or(Failure::RequestFailed)?,
        state,
        draft: source["draft"].as_bool().unwrap_or(false),
        updated_at: value(source, "/updated_at", 64).ok_or(Failure::RequestFailed)?,
        head_branch: None,
        head_repository: None,
        base_branch: None,
    })
}

pub async fn discover(
    remote: &Remote,
    filters: &PullDiscovery,
    page: u32,
) -> Result<Response, Failure> {
    discover_with_cli("gh", remote, filters, page).await
}

async fn discover_with_cli(
    program: &str,
    remote: &Remote,
    filters: &PullDiscovery,
    page: u32,
) -> Result<Response, Failure> {
    if page > 1 && filters.account.is_none() {
        return Err(Failure::InvalidRequest);
    }
    let branches = filters.relation == "branches";
    // Validate before executing even the identity request.
    if branches {
        branches_endpoint(remote, filters, page)?;
    } else {
        query(remote, filters, "validation", page)?;
    }
    let user = json(program, remote, "user")
        .await
        .map_err(|_| Failure::ConnectionUnavailable)?;
    let account = value(&user, "/login", 100)
        .filter(|s| segment(s))
        .ok_or(Failure::ConnectionUnavailable)?;
    if filters
        .account
        .as_ref()
        .is_some_and(|expected| expected != &account)
    {
        return Err(Failure::ConnectionUnavailable);
    }
    let endpoint = if branches {
        branches_endpoint(remote, filters, page)?
    } else {
        query(remote, filters, &account, page)?
    };
    let data = json(program, remote, &endpoint)
        .await
        .map_err(|_| Failure::RequestFailed)?;
    let (rows, total, incomplete, more) = if branches {
        let rows = data
            .as_array()
            .filter(|rows| rows.len() <= BRANCH_PAGE_SIZE)
            .ok_or(Failure::RequestFailed)?;
        // This endpoint reports no total; a full page may have a successor,
        // and stopping at the page bound leaves the listing incomplete.
        let full = rows.len() == BRANCH_PAGE_SIZE;
        let seen = u64::from(page - 1) * BRANCH_PAGE_SIZE as u64 + rows.len() as u64;
        (
            rows,
            seen,
            full && page == BRANCH_PAGES,
            full && page < BRANCH_PAGES,
        )
    } else {
        let total = data["total_count"].as_u64().ok_or(Failure::RequestFailed)?;
        let incomplete = data["incomplete_results"]
            .as_bool()
            .ok_or(Failure::RequestFailed)?;
        let rows = data["items"]
            .as_array()
            .filter(|rows| rows.len() <= PAGE_SIZE as usize)
            .ok_or(Failure::RequestFailed)?;
        let more = !rows.is_empty() && u64::from(page) * PAGE_SIZE < total.min(1000);
        (rows, total, incomplete, more)
    };
    let items = rows
        .iter()
        .map(|row| {
            if branches {
                branch_summary(row, remote)
            } else {
                summary(row, remote)
            }
        })
        .collect::<Result<Vec<_>, _>>()?;
    // Guard account switching during the observation and across pagination.
    let after = json(program, remote, "user")
        .await
        .map_err(|_| Failure::ConnectionUnavailable)?;
    if value(&after, "/login", 100).as_ref() != Some(&account) {
        return Err(Failure::ConnectionUnavailable);
    }
    let next_page = more.then_some(page + 1);
    Ok(Response::Pulls {
        account,
        items,
        total,
        incomplete,
        next_page,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn remote() -> Remote {
        Remote {
            name: "origin".into(),
            host: "github.com".into(),
            owner: "workspace".into(),
            repository: "repo".into(),
        }
    }
    #[tokio::test]
    async fn discovery_cli_paginates_and_fences_account_switches() {
        use std::os::unix::fs::PermissionsExt as _;
        let root = tempfile::tempdir().unwrap();
        let program = root.path().join("gh-fixture");
        let user = root.path().join("user");
        let replacement = root.path().join("replacement");
        let calls = root.path().join("searched");
        std::fs::write(&user, r#"{"login":"alice"}"#).unwrap();
        std::fs::write(&program, format!(r#"#!/bin/sh
test -z "$GH_TOKEN$GITHUB_TOKEN$GH_DEBUG" || exit 41
test "$1:$2:$3:$4:$5:$6:$7" = 'api:--method:GET:--hostname:github.com:--header:Accept: application/vnd.github+json' || exit 42
case "$8" in
  user) cat '{}' ;;
  'search/issues?q=is%3Apr+author%3Aalice+is%3Aopen&sort=updated&order=desc&per_page=20&page=1')
    touch '{}'
    echo '{{"total_count":21,"incomplete_results":true,"items":[{{"number":7,"title":"Change","html_url":"https://github.com/another/project/pull/7","state":"open","user":{{"login":"alice"}},"updated_at":"2026-10-01T00:00:00Z","pull_request":{{}}}}]}}'
    if [ -f '{}' ]; then cp '{}' '{}'; fi ;;
  *) exit 43 ;;
esac
"#,user.display(),calls.display(),replacement.display(),replacement.display(),user.display())).unwrap();
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o700)).unwrap();
        let filters = PullDiscovery {
            relation: "author".into(),
            state: "open".into(),
            current_repository: false,
            account: None,
        };
        let response = discover_with_cli(program.to_str().unwrap(), &remote(), &filters, 1)
            .await
            .unwrap();
        let Response::Pulls {
            account,
            items,
            total,
            incomplete,
            next_page,
        } = response
        else {
            panic!("expected PRs");
        };
        assert_eq!(account, "alice");
        assert_eq!(items.len(), 1);
        assert_eq!(total, 21);
        assert!(incomplete);
        assert_eq!(next_page, Some(2));
        std::fs::remove_file(&calls).unwrap();
        let previous = PullDiscovery {
            account: Some("bob".into()),
            ..filters.clone()
        };
        assert!(matches!(
            discover_with_cli(program.to_str().unwrap(), &remote(), &previous, 1).await,
            Err(Failure::ConnectionUnavailable)
        ));
        assert!(!calls.exists());
        std::fs::write(&replacement, r#"{"login":"bob"}"#).unwrap();
        assert!(matches!(
            discover_with_cli(program.to_str().unwrap(), &remote(), &filters, 1).await,
            Err(Failure::ConnectionUnavailable)
        ));
    }

    #[tokio::test]
    async fn branches_relation_lists_open_pulls_with_their_branches() {
        use std::os::unix::fs::PermissionsExt as _;
        let root = tempfile::tempdir().unwrap();
        let program = root.path().join("gh-fixture");
        let row = |number: u32, head: &str, fork: bool| {
            serde_json::json!({
                "number": number,
                "title": "Change",
                "html_url": format!("https://github.com/workspace/repo/pull/{number}"),
                "state": "open",
                "draft": number == 8,
                "merged_at": null,
                "user": {"login": "alice"},
                "updated_at": "2026-10-01T00:00:00Z",
                "body": "private body",
                "head": {
                    "ref": head,
                    "repo": if fork { serde_json::Value::Null } else {
                        serde_json::json!({"full_name": "workspace/repo"})
                    },
                },
                "base": {"ref": "main"},
            })
        };
        let first = serde_json::Value::Array(
            (1..=10)
                .map(|n| row(n, &format!("feature/{n}"), false))
                .collect(),
        );
        let second = serde_json::json!([row(11, "from-a-deleted-fork", true)]);
        let pulls = "repos/workspace/repo/pulls?state=open&sort=updated&direction=desc&per_page=10";
        std::fs::write(
            &program,
            format!(
                "#!/bin/sh\ncase \"$8\" in\n  user) echo '{{\"login\":\"alice\"}}' ;;\n  '{pulls}&page=1') cat <<'JSON'\n{first}\nJSON\n ;;\n  '{pulls}&page=2') cat <<'JSON'\n{second}\nJSON\n ;;\n  *) exit 43 ;;\nesac\n"
            ),
        )
        .unwrap();
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o700)).unwrap();
        let filters = PullDiscovery {
            relation: "branches".into(),
            state: "open".into(),
            current_repository: true,
            account: None,
        };
        let workspace = remote();
        let page = |page| discover_with_cli(program.to_str().unwrap(), &workspace, &filters, page);
        let Response::Pulls {
            items,
            total,
            incomplete,
            next_page,
            ..
        } = page(1).await.unwrap()
        else {
            panic!("expected PRs");
        };
        assert_eq!(
            (items.len(), total, incomplete, next_page),
            (10, 10, false, Some(2))
        );
        assert_eq!(items[0].head_branch.as_deref(), Some("feature/1"));
        assert_eq!(items[0].head_repository.as_deref(), Some("workspace/repo"));
        assert_eq!(items[0].base_branch.as_deref(), Some("main"));
        assert!(items[7].draft);
        assert!(
            !serde_json::to_string(&items)
                .unwrap()
                .contains("private body")
        );
        let later = PullDiscovery {
            account: Some("alice".into()),
            ..filters.clone()
        };
        let Response::Pulls {
            items,
            total,
            next_page,
            ..
        } = discover_with_cli(program.to_str().unwrap(), &remote(), &later, 2)
            .await
            .unwrap()
        else {
            panic!("expected PRs");
        };
        assert_eq!((items.len(), total, next_page), (1, 11, None));
        assert_eq!(items[0].head_repository, None);

        // The relation is one fixed listing: nothing but this repository's
        // open pull requests, within the page bound.
        for (state, current_repository, page) in
            [("closed", true, 1), ("open", false, 1), ("open", true, 11)]
        {
            let refused = PullDiscovery {
                state: state.into(),
                current_repository,
                account: Some("alice".into()),
                ..filters.clone()
            };
            assert!(matches!(
                discover_with_cli(program.to_str().unwrap(), &remote(), &refused, page).await,
                Err(Failure::InvalidRequest)
            ));
        }
    }

    #[test]
    fn search_summaries_omit_branch_fields_on_the_wire() {
        let row = serde_json::json!({"number":7,"title":"Change","html_url":"https://github.com/another/project/pull/7","state":"open","user":{"login":"alice"},"updated_at":"2026-10-01T00:00:00Z","pull_request":{}});
        let wire = serde_json::to_value(summary(&row, &remote()).unwrap()).unwrap();
        // A Controller that predates the fields rejects unknown members.
        assert!(
            wire.as_object()
                .unwrap()
                .keys()
                .all(|key| !key.contains("Branch") && key != "headRepository")
        );
    }

    #[test]
    fn discovery_queries_are_closed_and_targets_remain_on_workspace_host() {
        let mut filters = PullDiscovery {
            relation: "author".into(),
            state: "open".into(),
            current_repository: false,
            account: None,
        };
        let endpoint = query(&remote(), &filters, "alice", 1).unwrap();
        let params = url::form_urlencoded::parse(endpoint.split_once('?').unwrap().1.as_bytes())
            .collect::<std::collections::BTreeMap<_, _>>();
        assert_eq!(params["q"], "is:pr author:alice is:open");
        filters.relation = "review".into();
        filters.current_repository = true;
        assert!(
            query(&remote(), &filters, "alice", 2)
                .unwrap()
                .contains("review-requested%3Aalice")
        );
        assert!(query(&remote(), &filters, "alice is:closed", 1).is_err());
        assert!(query(&remote(), &filters, "alice", 51).is_err());
        filters.relation = "author:bob".into();
        assert!(query(&remote(), &filters, "alice", 1).is_err());
        let selected = review_target(&remote(), Some("another/project")).unwrap();
        assert_eq!(selected.host, "github.com");
        assert_eq!(selected.owner, "another");
        for bad in [
            "../repo",
            "owner/repo/pulls",
            "https://evil.test/repo",
            "owner/repo?x=1",
        ] {
            assert!(review_target(&remote(), Some(bad)).is_err());
        }
    }
    #[test]
    fn discovery_rejects_foreign_urls_and_projects_only_summary_fields() {
        let mut row = serde_json::json!({"number":7,"title":"Change","html_url":"https://github.com/another/project/pull/7","state":"open","draft":true,"user":{"login":"alice"},"updated_at":"2026-10-01T00:00:00Z","pull_request":{},"body":"private body"});
        let item = summary(&row, &remote()).unwrap();
        assert_eq!(item.repository, "another/project");
        assert!(item.draft);
        assert!(
            !serde_json::to_string(&item)
                .unwrap()
                .contains("private body")
        );
        row["html_url"] = serde_json::json!("https://evil.test/another/project/pull/7");
        assert!(summary(&row, &remote()).is_err());
    }
}
