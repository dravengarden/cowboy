//! `exec-confined -- <argv>`: the `ExecStart` wrapper of every account unit
//! (amendment R3-3). It runs as the account user, sets `PR_SET_NO_NEW_PRIVS`,
//! scopes the process tree away from abstract Unix sockets and signals outside
//! it with a Landlock ruleset that handles no filesystem or network access,
//! and then execs the fixed command. It refuses to exec when the kernel lacks
//! Landlock ABI 6.

use std::os::unix::process::CommandExt as _;
use std::process::Command;

use anyhow::{Context as _, Result, bail};

use crate::raw;

/// The minimum Landlock ABI that knows IPC scoping.
pub const MIN_LANDLOCK_ABI: u32 = 6;

/// Parse `-- <program> [args...]`.
pub fn parse(arguments: &[String]) -> Result<&[String]> {
    match arguments.split_first() {
        Some((separator, rest)) if separator == "--" && !rest.is_empty() => Ok(rest),
        _ => bail!("expected `-- <program> [args...]`"),
    }
}

/// Confine the current process and exec `argv`; returns only on failure.
pub fn run(argv: &[String]) -> anyhow::Error {
    match confine_and_exec(argv) {
        Ok(never) => match never {},
        Err(error) => error,
    }
}

enum Never {}

fn confine_and_exec(argv: &[String]) -> Result<Never> {
    rustix::thread::set_no_new_privs(true).context("PR_SET_NO_NEW_PRIVS")?;
    let abi = raw::landlock_abi().context("Landlock is unavailable")?;
    if abi < MIN_LANDLOCK_ABI {
        bail!("Landlock ABI {abi} is older than the required {MIN_LANDLOCK_ABI}");
    }
    let ruleset = raw::scoped_ruleset(raw::SCOPE_ABSTRACT_UNIX_SOCKET | raw::SCOPE_SIGNAL)
        .context("landlock_create_ruleset")?;
    raw::restrict_self(&ruleset).context("landlock_restrict_self")?;
    drop(ruleset);
    let error = Command::new(&argv[0]).args(&argv[1..]).exec();
    Err(error).with_context(|| format!("execve {}", argv[0]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_requires_separator_and_program() {
        let ok = vec!["--".to_owned(), "/bin/true".to_owned(), "x".to_owned()];
        assert_eq!(parse(&ok).unwrap(), &ok[1..]);
        assert!(parse(&["--".to_owned()]).is_err());
        assert!(parse(&["/bin/true".to_owned()]).is_err());
        assert!(parse(&[]).is_err());
    }
}
