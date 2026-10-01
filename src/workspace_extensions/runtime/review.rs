//! Read-only PR observations. Every page checks repository and both refs before
//! and after the request. This detects concurrent updates, not an API transaction.
use serde_json::Value;
use sha2::{Digest, Sha256};

use super::{command, value};
use crate::workspace_extensions::{Failure, Remote, Response, ReviewFile, ReviewPage, ReviewRead};

const PAGE_SIZE: u64 = 20;
const FILE_LIMIT: u64 = 3000;

fn numeric(value: &str) -> bool {
    !value.is_empty() && value.len() <= 24 && value.bytes().all(|b| b.is_ascii_digit())
}

fn oid(source: &Value, pointer: &str) -> Result<String, Failure> {
    let text = value(source, pointer, 64).ok_or(Failure::ReviewUnavailable)?;
    if !matches!(text.len(), 40 | 64) || !text.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(Failure::ReviewUnavailable);
    }
    Ok(text)
}

fn metadata(source: &Value, remote: &Remote, number: &str) -> Result<ReviewPage, Failure> {
    let repository_id = value(source, "/base/repo/id", 24)
        .filter(|id| numeric(id))
        .ok_or(Failure::ReviewUnavailable)?;
    let actual_number = value(source, "/number", 24).ok_or(Failure::ReviewUnavailable)?;
    let full_name = value(source, "/base/repo/full_name", 201).ok_or(Failure::ReviewUnavailable)?;
    if actual_number != number
        || !full_name.eq_ignore_ascii_case(&format!("{}/{}", remote.owner, remote.repository))
    {
        return Err(Failure::RepositoryUnavailable);
    }
    let head = oid(source, "/head/sha")?;
    let base = oid(source, "/base/sha")?;
    let base_ref = value(source, "/base/ref", 1024).ok_or(Failure::ReviewUnavailable)?;
    let mut hash = Sha256::new();
    for part in [&repository_id, number, &head, &base, &base_ref] {
        hash.update(part.as_bytes());
        hash.update([0]);
    }
    let total_files = source["changed_files"]
        .as_u64()
        .ok_or(Failure::ReviewUnavailable)?;
    let state = if source["merged"].as_bool() == Some(true) {
        "merged".to_owned()
    } else {
        value(source, "/state", 16)
            .filter(|s| matches!(s.as_str(), "open" | "closed"))
            .ok_or(Failure::ReviewUnavailable)?
    };
    Ok(ReviewPage {
        repository_id,
        number: number.to_owned(),
        title: value(source, "/title", 1000).ok_or(Failure::ReviewUnavailable)?,
        url: format!(
            "https://{}/{}/{}/pull/{number}",
            remote.host, remote.owner, remote.repository
        ),
        state,
        head,
        base,
        revision: format!("{:x}", hash.finalize()),
        total_files,
        files: Vec::new(),
        next_page: None,
        limited: total_files > FILE_LIMIT,
    })
}

fn file(source: &Value) -> Result<ReviewFile, Failure> {
    let path = value(source, "/filename", 4096).ok_or(Failure::ReviewUnavailable)?;
    let status = value(source, "/status", 32).ok_or(Failure::ReviewUnavailable)?;
    let additions = source["additions"]
        .as_u64()
        .ok_or(Failure::ReviewUnavailable)?;
    let deletions = source["deletions"]
        .as_u64()
        .ok_or(Failure::ReviewUnavailable)?;
    let patch = value(source, "/patch", 256 * 1024);
    // API patches contain hunks, not file headers. Counting every +/- row also
    // counts source lines beginning with +++/--- correctly.
    let (added, removed) = patch.as_ref().map_or((0, 0), |patch| {
        patch.lines().fold((0_u64, 0_u64), |(a, d), line| {
            (
                a + u64::from(line.starts_with('+')),
                d + u64::from(line.starts_with('-')),
            )
        })
    });
    let limited = added != additions || removed != deletions;
    Ok(ReviewFile {
        path,
        old_path: value(source, "/previous_filename", 4096),
        status,
        additions,
        deletions,
        patch,
        limited,
    })
}

async fn json(program: &str, remote: &Remote, endpoint: &str) -> Result<Value, Failure> {
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
            endpoint,
        ]
        .map(str::to_owned),
    )
    .await
    .map_err(|_| Failure::ReviewUnavailable)?;
    serde_json::from_slice(&bytes).map_err(|_| Failure::ReviewUnavailable)
}

pub(crate) async fn read_review(
    remote: &Remote,
    number: &str,
    read: &ReviewRead,
    page: u32,
) -> Result<Response, Failure> {
    read_with_cli("gh", remote, number, read, page).await
}

async fn read_with_cli(
    program: &str,
    remote: &Remote,
    number: &str,
    read: &ReviewRead,
    page: u32,
) -> Result<Response, Failure> {
    if !numeric(number)
        || page == 0
        || u64::from(page) > FILE_LIMIT / PAGE_SIZE
        || (page > 1 && (read.repository_id.is_none() || read.revision.is_none()))
    {
        return Err(Failure::InvalidRequest);
    }
    command(
        program,
        &["auth", "status", "--active", "--hostname", &remote.host].map(str::to_owned),
    )
    .await
    .map_err(|_| Failure::ConnectionUnavailable)?;
    let endpoint = format!(
        "repos/{}/{}/pulls/{number}",
        remote.owner, remote.repository
    );
    let mut before = metadata(&json(program, remote, &endpoint).await?, remote, number)?;
    if read
        .repository_id
        .as_ref()
        .is_some_and(|id| *id != before.repository_id)
    {
        return Err(Failure::RepositoryUnavailable);
    }
    if read
        .revision
        .as_ref()
        .is_some_and(|revision| *revision != before.revision)
    {
        return Err(Failure::ReviewChanged);
    }
    let offset = (u64::from(page) - 1) * PAGE_SIZE;
    if offset > 0 && offset >= before.total_files {
        return Err(Failure::InvalidRequest);
    }
    let data = json(
        program,
        remote,
        &format!("{endpoint}/files?per_page={PAGE_SIZE}&page={page}"),
    )
    .await?;
    let rows = data.as_array().ok_or(Failure::ReviewUnavailable)?;
    if rows.len() as u64 != before.total_files.saturating_sub(offset).min(PAGE_SIZE) {
        return Err(Failure::ReviewChanged);
    }
    before.files = rows.iter().map(file).collect::<Result<_, _>>()?;
    let after = metadata(&json(program, remote, &endpoint).await?, remote, number)?;
    if before.revision != after.revision || before.total_files != after.total_files {
        return Err(Failure::ReviewChanged);
    }
    if offset + PAGE_SIZE < before.total_files.min(FILE_LIMIT) {
        before.next_page = Some(page + 1);
    }
    Ok(Response::Review { review: before })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn pr() -> Value {
        json!({"number":12,"title":"PR","state":"open","merged":false,"changed_files":1,
            "base":{"repo":{"id":123,"full_name":"owner/repo"},"sha":"a".repeat(40),"ref":"main"},
            "head":{"sha":"b".repeat(40)}})
    }

    #[tokio::test]
    async fn cli_pages_reject_force_push_and_never_cross_repository_identity() {
        use std::os::unix::fs::PermissionsExt as _;
        let root = tempfile::tempdir().unwrap();
        let program = root.path().join("gh-fixture");
        let metadata_path = root.path().join("meta.json");
        let files_path = root.path().join("files.json");
        let replacement = root.path().join("replace.json");
        let requested = root.path().join("requested");
        std::fs::write(&metadata_path, pr().to_string()).unwrap();
        std::fs::write(&files_path, json!([{"filename":"a.rs","status":"modified","additions":1,"deletions":0,"patch":"@@ -0,0 +1 @@\n+hi"}]).to_string()).unwrap();
        std::fs::write(&program, format!(r#"#!/bin/sh
test -z "$GH_TOKEN$GITHUB_TOKEN$GH_DEBUG" || exit 41
if [ "$1" = auth ]; then exit 0; fi
test "$1:$2:$3:$4:$5:$6:$7" = 'api:--method:GET:--hostname:github.com:--header:Accept: application/vnd.github+json' || exit 42
case "$8" in
  'repos/owner/repo/pulls/12') cat '{}' ;;
  'repos/owner/repo/pulls/12/files?per_page=20&page=1')
    touch '{}'
    cat '{}'
    if [ -f '{}' ]; then cp '{}' '{}'; fi ;;
  *) exit 43 ;;
esac
"#, metadata_path.display(), requested.display(), files_path.display(), replacement.display(), replacement.display(), metadata_path.display())).unwrap();
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o700)).unwrap();
        let remote =
            super::super::normalize_remote("origin", "git@github.com:owner/repo.git").unwrap();
        let read = ReviewRead {
            repository_id: Some("123".into()),
            revision: None,
        };
        let result = read_with_cli(program.to_str().unwrap(), &remote, "12", &read, 1)
            .await
            .unwrap();
        let Response::Review { review } = result else {
            panic!("review expected");
        };
        assert_eq!(review.files.len(), 1);
        std::fs::remove_file(&requested).unwrap();
        let wrong = ReviewRead {
            repository_id: Some("124".into()),
            revision: None,
        };
        assert!(matches!(
            read_with_cli(program.to_str().unwrap(), &remote, "12", &wrong, 1).await,
            Err(Failure::RepositoryUnavailable)
        ));
        assert!(!requested.exists());
        let mut updated = pr();
        updated["head"]["sha"] = json!("c".repeat(40));
        std::fs::write(replacement, updated.to_string()).unwrap();
        assert!(matches!(
            read_with_cli(program.to_str().unwrap(), &remote, "12", &read, 1).await,
            Err(Failure::ReviewChanged)
        ));
        let old = ReviewRead {
            repository_id: Some("123".into()),
            revision: Some(review.revision),
        };
        std::fs::remove_file(&requested).unwrap();
        assert!(matches!(
            read_with_cli(program.to_str().unwrap(), &remote, "12", &old, 1).await,
            Err(Failure::ReviewChanged)
        ));
        assert!(!requested.exists());
        assert!(matches!(
            read_with_cli(program.to_str().unwrap(), &remote, "../12", &read, 1).await,
            Err(Failure::InvalidRequest)
        ));
    }

    #[test]
    fn snapshot_identity_includes_repository_base_head_and_retarget() {
        let remote =
            super::super::normalize_remote("origin", "git@github.com:owner/repo.git").unwrap();
        let original = metadata(&pr(), &remote, "12").unwrap();
        for (pointer, replacement) in [
            ("/head/sha", json!("c".repeat(40))),
            ("/base/sha", json!("d".repeat(40))),
            ("/base/ref", json!("release")),
            ("/base/repo/id", json!(124)),
        ] {
            let mut changed = pr();
            *changed.pointer_mut(pointer).unwrap() = replacement;
            assert_ne!(
                metadata(&changed, &remote, "12").unwrap().revision,
                original.revision
            );
        }
        assert!(metadata(&pr(), &remote, "13").is_err());
        let mut foreign = pr();
        foreign["base"]["repo"]["full_name"] = json!("other/repo");
        assert!(metadata(&foreign, &remote, "12").is_err());
    }

    #[test]
    fn incomplete_and_binary_patches_are_never_claimed_complete() {
        let mut row = json!({"filename":"odd[1].rs","status":"renamed","previous_filename":"old.rs",
            "additions":1,"deletions":1,"patch":"@@ -1 +1 @@\n--- source\n+++ source"});
        assert!(!file(&row).unwrap().limited);
        row["additions"] = json!(2);
        assert!(file(&row).unwrap().limited);
        row["patch"] = Value::Null;
        assert!(file(&row).unwrap().limited);
    }
}
