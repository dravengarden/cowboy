use crate::runtime_wire::CoreCommand;

/// The acknowledgement that refuses `command` when the core queue is full.
/// A deletion has none: the controller forgets a stopped session's declaration
/// even on refusal, so it keeps the reconnect path and its full replay.
pub(super) fn overflow_refusal(command: &CoreCommand) -> Option<(String, String)> {
    match command {
        CoreCommand::RecoverExecutionSession {
            session,
            command_id,
            ..
        }
        | CoreCommand::ReadmitAccountSession {
            session,
            command_id,
            ..
        } => Some((session.session_id.clone(), command_id.clone())),
        CoreCommand::EnsureSession {
            session,
            command_id,
        } => Some((
            session.session_id.clone(),
            command_id
                .clone()
                .unwrap_or_else(|| format!("ensure:{}", session.session_id)),
        )),
        CoreCommand::StopSession {
            session_id,
            command_id,
        } if command_id.starts_with("reset-") => Some((session_id.clone(), command_id.clone())),
        CoreCommand::CancelTurn {
            session_id,
            command_id,
            ..
        }
        | CoreCommand::Steer {
            session_id,
            command_id,
            ..
        }
        | CoreCommand::Prompt {
            session_id,
            command_id,
            ..
        }
        | CoreCommand::Cancel {
            session_id,
            command_id,
        }
        | CoreCommand::CancelSubagent {
            session_id,
            command_id,
            ..
        }
        | CoreCommand::Permission {
            session_id,
            command_id,
            ..
        }
        | CoreCommand::SetConfigOption {
            session_id,
            command_id,
            ..
        }
        | CoreCommand::HibernateSession {
            session_id,
            command_id,
            ..
        }
        | CoreCommand::QuiesceAccountSession {
            session_id,
            command_id,
            ..
        }
        | CoreCommand::DrainCrashedWorker {
            session_id,
            command_id,
            ..
        } => Some((session_id.clone(), command_id.clone())),
        CoreCommand::WrapUpHint { .. }
        | CoreCommand::StopSession { .. }
        | CoreCommand::DrainSession { .. }
        | CoreCommand::SetDesiredGeneration { .. }
        | CoreCommand::RollProvider { .. } => None,
    }
}
