//! Review's Git commands start no program the repository configures.
//!
//! Repository configuration is writable by the session's agent, and Review
//! also reads contexts nobody executes in (other worktrees, project
//! checkouts). Git would otherwise start an fsmonitor daemon on every status,
//! hooks, signature verification, external diff and textconv drivers, and
//! clean/smudge/process filters named by `.gitattributes`. It would also
//! fetch a missing object of a partial clone from its promisor remote, through
//! whatever transport that remote configures, and compare a nested
//! repository's worktree in a child process under that repository's own
//! configuration. Review needs none of them: it reports what Git stores, not
//! what a project program renders.
//!
//! Filter drivers have no command-line switch, so commands that read worktree
//! files first list the filter keys the repository configures (a pure config
//! read) and blank each one. A writer racing that read can still name a new
//! driver for the following command. A file whose filter rewrites it is
//! therefore read as its plain bytes, and one whose filter is `required`
//! fails to read. A nested repository is reported only when its checked-out
//! commit moved: its worktree is never compared, so its own filters and
//! drivers are never consulted, and an `ignore` setting that would hide it is
//! overridden too. Lazy fetching is switched off twice, because a Git that
//! predates `GIT_NO_LAZY_FETCH` ignores it: no command but the fetch is
//! allowed any transport.
//!
//! A fetch cannot avoid its transport, so it runs the programs that user and
//! system configuration choose and none that the repository's own scopes
//! name; see [`git_fetch`]. `remote.<name>.vcs` is the one setting it cannot
//! outrank: it can only select a helper named after an allowed protocol,
//! which Git ships for HTTPS alone.

use std::ffi::OsString;
use std::io::Read as _;
use std::os::unix::ffi::OsStringExt as _;
use std::path::Path;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::time::Instant;

const GIT_FETCH_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

/// The trusted transport settings of a host are a handful of short lines.
const MAX_TRANSPORT_SETTINGS_BYTES: u64 = 1024 * 1024;

/// One configuration setting in command scope. Keys are Git's own bytes: a
/// driver name need not be text, and a rewritten key would override nothing.
type Setting = (OsString, OsString);

/// Overrides every invocation carries, in Git's unambiguous environment form.
const FIXED: [(&str, &str); 4] = [
    ("core.fsmonitor", "false"),
    ("core.hookspath", "/dev/null"),
    ("core.pager", "cat"),
    ("log.showsignature", "false"),
];

/// Subcommands that render diffs and so would run diff drivers. A configured
/// `diff.submodule=diff` would also render a nested repository's changes in a
/// child process that honors that repository's drivers.
const DIFF_RENDERING: [&str; 3] = ["diff", "show", "log"];

/// Subcommands that read worktree files through Git's conversion and so would
/// run filters.
const WORKTREE_READING: [&str; 3] = ["diff", "status", "hash-object"];

/// Subcommands that would compare a nested repository's worktree in a child
/// process, under that repository's own filters.
const SUBMODULE_COMPARING: [&str; 2] = ["diff", "status"];

/// Settings that name a program Git starts while it fetches: the SSH command
/// and its askpass, the `git://` proxy, credential helpers, the program that
/// serves the pack and the alternate-refs lister.
const TRANSPORT_PROGRAM_KEYS: &str = r"^(core\.(sshcommand|askpass|gitproxy|alternaterefscommand)|credential\.(.+\.)?helper|remote\..+\.uploadpack)$";

fn command(root: &Path, overrides: &[Setting]) -> Command {
    let mut command = Command::new("git");
    command
        .current_dir(root)
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("GIT_TERMINAL_PROMPT", "0")
        // A repository marked as a partial clone would otherwise fetch a
        // missing object in the middle of a read, through the transport
        // programs its promisor remote configures. Only the fetch names the
        // protocols it may use.
        .env("GIT_NO_LAZY_FETCH", "1")
        .env("GIT_ALLOW_PROTOCOL", "")
        .env_remove("GIT_EXTERNAL_DIFF")
        .env(
            "GIT_CONFIG_COUNT",
            (FIXED.len() + overrides.len()).to_string(),
        );
    let fixed = FIXED
        .iter()
        .map(|(key, value)| (OsString::from(key), OsString::from(value)));
    for (index, (key, value)) in fixed.chain(overrides.iter().cloned()).enumerate() {
        command
            .env(format!("GIT_CONFIG_KEY_{index}"), key)
            .env(format!("GIT_CONFIG_VALUE_{index}"), value);
    }
    command
}

/// Waits for `child` until `deadline`; a child still running then is killed
/// and reported as `None`.
fn wait_until(child: &mut Child, deadline: Instant) -> Result<Option<ExitStatus>, String> {
    loop {
        match child.try_wait().map_err(|error| error.to_string())? {
            Some(status) => return Ok(Some(status)),
            None if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                return Ok(None);
            }
            None => std::thread::sleep(std::time::Duration::from_millis(100)),
        }
    }
}

/// Configured filter-driver keys, each blanked so Git treats the file as
/// unfiltered. Keys come from Git itself, including included files.
fn filter_overrides(root: &Path) -> Result<Vec<Setting>, String> {
    let output = command(root, &[])
        // A driver name need not be text, and under a UTF-8 locale the
        // pattern does not match bytes that are not.
        .env("LC_ALL", "C")
        .args([
            "config",
            "--null",
            "--name-only",
            "--get-regexp",
            r"^filter\..+\.(clean|smudge|process)$",
        ])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .map_err(|error| format!("starting git: {error}"))?;
    // Exit 1 means no key matched.
    if !output.status.success() && output.status.code() != Some(1) {
        return Err("reading git configuration failed".to_owned());
    }
    Ok(output
        .stdout
        .split(|byte| *byte == 0)
        .filter(|key| !key.is_empty())
        .map(|key| (OsString::from_vec(key.to_vec()), OsString::new()))
        .collect())
}

/// What user and system configuration say about [`TRANSPORT_PROGRAM_KEYS`],
/// in the order Git reads them. The repository's own scopes are left out:
/// they are agent-writable. The listing reads every configuration file the
/// repository includes, so it shares the fetch's deadline.
fn trusted_transport_settings(
    root: &Path,
    deadline: Instant,
) -> Result<Vec<(String, String)>, String> {
    let mut child = command(root, &[])
        // Subsections (remote names, credential URLs) need not be text.
        .env("LC_ALL", "C")
        .args([
            "config",
            "--null",
            "--show-scope",
            "--includes",
            "--get-regexp",
            TRANSPORT_PROGRAM_KEYS,
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| format!("starting git: {error}"))?;
    let stdout = child.stdout.take().ok_or("missing git stdout")?;
    // Reading beside the wait keeps a full pipe from stalling the child.
    let reader = std::thread::spawn(move || {
        let mut listed = Vec::new();
        let _ = stdout
            .take(MAX_TRANSPORT_SETTINGS_BYTES)
            .read_to_end(&mut listed);
        listed
    });
    let status = wait_until(&mut child, deadline)?;
    let listed = reader
        .join()
        .map_err(|_| "reading git configuration failed".to_owned())?;
    let Some(status) = status else {
        return Err("reading git configuration timed out".to_owned());
    };
    // Exit 1 means no key matched.
    if (!status.success() && status.code() != Some(1))
        || listed.len() as u64 >= MAX_TRANSPORT_SETTINGS_BYTES
    {
        return Err("reading git configuration failed".to_owned());
    }
    Ok(trusted_settings(&listed))
}

/// The system and global records of `git config --null --show-scope`
/// output. Each record is `scope NUL key LF value NUL`; a valueless key has
/// no LF.
fn trusted_settings(listed: &[u8]) -> Vec<(String, String)> {
    let mut fields = listed.split(|byte| *byte == 0);
    let mut settings = Vec::new();
    while let (Some(scope), Some(entry)) = (fields.next(), fields.next()) {
        if !matches!(scope, b"system" | b"global") {
            continue;
        }
        let entry = String::from_utf8_lossy(entry);
        let (key, value) = entry.split_once('\n').unwrap_or((&entry, ""));
        settings.push((key.to_owned(), value.to_owned()));
    }
    settings
}

/// `value` as one word of a POSIX shell command.
fn shell_word(value: &str) -> String {
    format!("'{}'", value.replace('\'', r"'\''"))
}

/// How one fetch pins every program its transport can start.
#[derive(Debug, PartialEq, Eq)]
struct Transport {
    /// Command-scope configuration, which Git reads after every file.
    overrides: Vec<(String, String)>,
    upload_pack: String,
    environment: Vec<(&'static str, String)>,
    protocols: &'static str,
}

/// The transport programs user and system configuration choose for `remote`,
/// in forms that outrank the repository's scopes whatever they hold when Git
/// reads them: nothing here depends on an earlier look at the repository.
///
/// - The environment outranks `core.sshCommand`, `core.askPass` and
///   `core.gitProxy`. A variable the Controller already carries stays as it
///   is and so decides before any configuration. Proxy rules match per host
///   and the first match wins, so a trusted rule cannot be pinned for every
///   host: with one configured, `git://` is not fetched at all.
/// - Git keeps the first `remote.<name>.uploadpack` it reads, so only the
///   command line outranks the repository.
/// - Command-scope configuration replaces the alternate-refs lister, and its
///   empty `credential.helper` drops every helper named before it, after which
///   the trusted ones are named again in their order.
fn transport(
    trusted: &[(String, String)],
    remote: &str,
    environment: impl Fn(&str) -> Option<String>,
) -> Transport {
    let last = |key: &str| {
        trusted
            .iter()
            .rev()
            .find(|(candidate, _)| candidate == key)
            .map(|(_, value)| value.clone())
    };
    let mut overrides = vec![("credential.helper".to_owned(), String::new())];
    overrides.extend(
        trusted
            .iter()
            .filter(|(key, _)| key.starts_with("credential."))
            .cloned(),
    );
    overrides.push((
        "core.alternaterefscommand".to_owned(),
        // Without one, advertising no alternate tips only costs negotiation.
        last("core.alternaterefscommand").unwrap_or_else(|| "true".to_owned()),
    ));
    let upload_pack_key = format!("remote.{remote}.uploadpack");
    let upload_pack = trusted
        .iter()
        .find(|(key, _)| *key == upload_pack_key)
        .map_or_else(|| "git-upload-pack".to_owned(), |(_, value)| value.clone());

    let mut pinned = Vec::new();
    if environment("GIT_SSH_COMMAND").is_none() {
        let ssh = last("core.sshcommand")
            .or_else(|| environment("GIT_SSH").map(|program| shell_word(&program)))
            .unwrap_or_else(|| "ssh".to_owned());
        pinned.push(("GIT_SSH_COMMAND", ssh));
    }
    if environment("GIT_ASKPASS").is_none() {
        // An empty value still outranks the configuration, and asks nothing.
        let askpass = last("core.askpass")
            .or_else(|| environment("SSH_ASKPASS"))
            .unwrap_or_default();
        pinned.push(("GIT_ASKPASS", askpass));
    }
    let mut protocols = "https:ssh:git:file";
    if environment("GIT_PROXY_COMMAND").is_none() {
        if trusted.iter().any(|(key, _)| key == "core.gitproxy") {
            // Proxy rules match per host and the first match wins, so a
            // repository rule could still be chosen for a host the trusted
            // rules leave out. `git://` is then not fetched from Review.
            protocols = "https:ssh:file";
        } else {
            pinned.push(("GIT_PROXY_COMMAND", String::new()));
        }
    }
    Transport {
        overrides,
        upload_pack,
        environment: pinned,
        protocols,
    }
}

/// A bounded, non-interactive fetch over the repository's configured remote.
///
/// Repository config is agent-writable, so the programs the transport starts
/// are the ones [`transport`] pins. `ext::` and other helper transports stay
/// refused by protocol. Nothing but the named ref is written: pruning and
/// the maintenance Git would run afterwards are off whatever the repository
/// configures.
pub(super) fn git_fetch(root: &Path, remote: &str, refspec: &str) -> Result<(), String> {
    let deadline = Instant::now() + GIT_FETCH_TIMEOUT;
    let transport = transport(
        &trusted_transport_settings(root, deadline)?,
        remote,
        |name| std::env::var(name).ok(),
    );
    let overrides: Vec<Setting> = transport
        .overrides
        .iter()
        .map(|(key, value)| (key.into(), value.into()))
        .collect();
    let mut child = command(root, &overrides)
        .args([
            "fetch",
            "--quiet",
            "--no-tags",
            "--no-prune",
            "--no-prune-tags",
            "--no-write-fetch-head",
            "--no-auto-maintenance",
        ])
        .arg(format!("--upload-pack={}", transport.upload_pack))
        // Without an empty refmap, Git also moves the remote-tracking ref.
        .args([
            "--refmap=",
            "--no-recurse-submodules",
            "--",
            remote,
            refspec,
        ])
        .envs(transport.environment)
        .env("GIT_ALLOW_PROTOCOL", transport.protocols)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| format!("starting git: {error}"))?;
    match wait_until(&mut child, deadline)? {
        Some(status) if status.success() => Ok(()),
        Some(_) => Err(format!("could not fetch from {remote}")),
        None => Err(format!("fetching from {remote} timed out")),
    }
}

pub(super) fn git_output(root: &Path, args: &[&str], limit: usize) -> Result<Vec<u8>, String> {
    // Global options may come first; none of them takes a separate value.
    let position = args.iter().position(|argument| !argument.starts_with('-'));
    let subcommand = position.map_or("", |position| args[position]);
    let overrides = if WORKTREE_READING.contains(&subcommand) {
        filter_overrides(root)?
    } else {
        Vec::new()
    };
    let mut command = command(root, &overrides);
    // Review paths identify exact files, including names such as [id].tsx
    // or :(glob)*.txt. They are never Git pathspec patterns or magic.
    command.arg("--literal-pathspecs");
    let mut switches = Vec::new();
    if DIFF_RENDERING.contains(&subcommand) {
        switches.extend(["--no-ext-diff", "--no-textconv", "--submodule=short"]);
    }
    if SUBMODULE_COMPARING.contains(&subcommand) {
        switches.push("--ignore-submodules=dirty");
    }
    match position {
        Some(position) if !switches.is_empty() => {
            command
                .args(&args[..=position])
                .args(switches)
                .args(&args[position + 1..]);
        }
        _ => {
            command.args(args);
        }
    }
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("starting git: {error}"))?;
    let mut stdout = child.stdout.take().ok_or("missing git stdout")?;
    let mut bytes = Vec::new();
    stdout
        .by_ref()
        .take(limit as u64)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("reading git output: {error}"))?;
    drop(stdout);
    let output = child
        .wait_with_output()
        .map_err(|error| format!("waiting for git: {error}"))?;
    if output.status.success() || bytes.len() == limit {
        Ok(bytes)
    } else {
        Err(String::from_utf8_lossy(&output.stderr).trim().to_owned())
    }
}

/// Whether Git output bytes contain `needle`.
pub(super) fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

pub(super) fn git_path_is_tracked(root: &Path, path: &str) -> bool {
    git_output(root, &["ls-files", "--error-unmatch", "--", path], 1024).is_ok()
}

pub(crate) mod blob;

#[cfg(test)]
mod tests;
