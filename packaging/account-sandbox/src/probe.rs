//! `probe_runtime`: the install-time check that a registered runtime's
//! vendor launcher runs, executed in a confined unit of a daemon-owned probe
//! identity. No Cowboy account, credential or Machine-user process is
//! involved, so a vendor binary of an isolated Provider never runs outside a
//! unit.

use std::sync::Arc;
use std::time::Duration;

use serde_json::json;

use crate::daemon::{Conn, Daemon, Reply};
use crate::ids;
use crate::protocol::{Code, Mode, Outcome, fail};
use crate::start::StartRequest;

/// The reserved service of the probe identity; `ensure_account` refuses it.
pub const PROBE_SERVICE: &str = "cowboy-account-sandbox";
const PROBE_ACCOUNT: &str = "runtime-probe";
/// Bound of one probe unit's run.
const PROBE_TIMEOUT: Duration = Duration::from_secs(30);

impl Daemon {
    pub(crate) fn probe_runtime(
        self: &Arc<Self>,
        conn: &Conn,
        runtime_sha256: &str,
    ) -> Outcome<Reply> {
        self.ensure_account(PROBE_SERVICE, PROBE_ACCOUNT)?;
        let key = ids::derive_key(PROBE_SERVICE, PROBE_ACCOUNT);
        let started = self.start(
            conn,
            &StartRequest {
                key: &key,
                mode: Mode::Probe,
                credential_generation: 0,
                runtime_sha256,
                session_id: None,
                descriptor_b64: None,
            },
        )?;
        let unit = started.value["unit"]
            .as_str()
            .ok_or_else(|| fail(Code::Internal, "probe unit has no name"))?
            .to_owned();
        let mut fds = started.fds.into_iter();
        // Closing stdin gives the probe EOF; its output is drained (and
        // discarded) so a chatty probe never blocks on a full pipe.
        drop(fds.next());
        for fd in fds.take(2) {
            std::thread::spawn(move || {
                let _ = std::io::copy(&mut std::fs::File::from(fd), &mut std::io::sink());
            });
        }
        let exit = self.wait_exit(&unit, PROBE_TIMEOUT);
        // Stop (on a timeout) and confirm the unit gone before answering.
        self.stop_unit(&unit)?;
        Ok(Reply::value(json!({
            "runtime_sha256": runtime_sha256,
            "unit": unit,
            "exit_code": exit,
            "passed": exit == Some(0),
        })))
    }
}
