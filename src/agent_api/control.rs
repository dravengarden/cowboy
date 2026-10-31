//! On-demand product capability catalog. Four MCP tools remain the whole
//! resident surface; adding an operation never adds a model-visible tool.
use serde_json::{Value, json};

pub struct Operation {
    pub name: &'static str,
    pub write: bool,
    pub input: &'static str,
}

macro_rules! operations {
    ($($name:literal, $write:literal, $input:literal;)+) => {
        pub const OPERATIONS: &[Operation] = &[$(Operation {name:$name,write:$write,input:$input},)+];
    }
}

operations! {
    "operations.receipt", false, "{operation:string}; full durable result for this parent; select/paginate large results";
    "configuration.service", false, "{}; persisted Service file revision/values; configuration.read('service') gives effective values";
    "configuration.service_set", true, "{expected_revision:string,changes:Record<string,unknown|null>}; owner role; dotted keys from configuration.schema('service'); null restores default; live fields apply, restart fields await restart";
    "plugins.catalog", false, "{}";
    "plugins.call", true, "{plugin:string,query?:{version:string,digest:string},body:unknown}; exact signed Plugin command contract from catalog; no generic shell";
    "plugins.install", true, "{machine:string,plugin:string,version:string,digest:string}; exact ready signed release; managed membership requires host CLI; inspect plugins.install_operations after uncertainty";
    "plugins.uninstall_plan", true, "{machine:string,plugin:string}; prepares an expiring owner-bound plan; no removal";
    "plugins.uninstall", true, "{machine:string,plugin:string,body:{plan_id:string,confirm_active_sessions?:boolean}}; read uninstall_plan first; explicitly confirm any affected sessions";
    "plugins.installations", false, "{machine:string}";
    "plugins.operations", false, "{machine:string,plugin:string}";
    "plugins.install_operations", false, "{machine:string,plugin:string}";
    "plugins.history", false, "{machine:string,plugin:string}; bounded lifecycle history";
    "plugins.machine_receipt", false, "{machine:string,plugin:string,operation:string}; observe the existing Machine operation; never repeats it";
    "plugins.recovery_assessment", false, "{machine:string,plugin:string,operation:string}; read-only recovery evidence";
    "machines.health", false, "{machine:string}";
    "machines.refresh", true, "{machine:string}; requests current read-only inventory; does not install or restart components";
    "machines.durable_state", false, "{machine:string}";
    "machines.maintenance", false, "{}";
    "machines.maintenance_set", true, "{machine:string,body:{memory_reclaim?:boolean|null,reclaim_on_capacity?:boolean|null,auto_update_idle_sessions?:boolean|null}}; null inherits Service config";
    "projects.list", false, "{machine:string}";
    "projects.workspaces", false, "{}; registered New Session choices";
    "projects.policies", false, "{}";
    "projects.placements", false, "{machine:string}";
    "projects.request", true, "{machine:string,body:{action:'list'}|{action:'discover',root:string}|{action:'adopt',expected_revision:string}|{action:'upsert',expected_revision:string,project:MachineWorkspace}|{action:'remove',expected_revision:string,id:string}}; removal only unregisters; list supplies revision/project shape";
    "projects.policy_set", true, "{machine:string,body:{expected_revision:string,policy:{agent_mode:'disabled'|'local'|'remote'|'either',hosts_projects:boolean,remote_targets:string[]|null},preferred:boolean}}; read projects.policies first";
    "folders.list", false, "{}";
    "folders.defaults", false, "{folder:string}";
    "folders.create", true, "{body:{name:string,parent?:string|null,defaults?:Record<string,string|null>}}; defaults keys: project,machine,runtime_machine,provider,preset,review_directory";
    "folders.defaults_set", true, "{folder:string,body:{values:Record<string,string|null>,inherit?:string[]}}; keys: project,machine,runtime_machine,provider,preset,review_directory; null blocks inheritance; inherit removes override";
    "tools.defaults", false, "{}";
    "tools.session", false, "{session_id:string}";
    "tools.defaults_set", true, "{provider:string,body:{settings:AgentTools|null}}; owner role; tools.defaults supplies shape and choices";
    "tools.session_set", true, "{session_id:string,body:SessionTools}; tools.session supplies override/effective/catalog; existing Tools policy validation applies";
    "sessions.create", true, "{body:{provider:string,provider_account_id?:string,machine_id:string,cwd?:string,runtime_machine_id?:string,folder_id?:string,preset?:string,initial_prompt?:string,context_sources?:string[]}}; creates an owned ordinary session on a managed worktree; optional same-owner context links, never copied credentials or native history";
    "sessions.context", false, "{session_id:string}; context links, revision, pending fresh-agent selection and last send receipt; no worker starts";
    "sessions.config_revert", false, "{session_id:string}; values the latest in-place changes (model, reasoning and other sessions.configure options) replaced since the last sent message; empty once a message is sent";
    "sessions.config_revert_restore", true, "{session_id:string}; Back: re-applies those values through the sessions.configure path and clears the record; refused once a message was sent";
    "sessions.context_set", true, "{session_id:string,body:{revision:number,sources:string[],next_agent?:{provider:string,provider_account_id:string,machine_id:string,cwd:string,runtime_machine_id?:string,preset?:string,inherit_settings?:boolean}|null,resolve_operation?:string}}; CAS staging only. Up to 32 same-owner sources, including none. Fresh agent defaults to current session as source, stays beside it in the same folder, and inherits compatible settings at send (inherit_settings defaults true); explicit preset overrides model/reasoning without widening permissions. Editable until send; reasoning/permissions use sessions.configure. No native history, credentials or file transfer. Only after inspecting an unresolved send, resolve_operation acknowledges that exact receipt with empty sources and no next_agent; preserves its destination, never resends";
    "sessions.context_send", true, "{session_id:string,body:{revision:number,operation:string,text:string,content?:unknown[]}}; first explicit send consumes staged context, creates a fresh isolated session only if next_agent was selected. Stable operation id is required. Source must be idle with no queued/background work. Returns destination and receipt; preparing/unknown means observe, never replay using a new id. All later messages use normal queue.submit";
    "sessions.duplicate", true, "{session_id:string,body:{operation:string,provider:string,provider_account_id:string,preset?:string,config?:object,inherit_settings?:boolean}}; a copy on another subscription or Provider (or where a native fork is unavailable): a new session on the source's Machines and project, placed beside it as SOURCE (copy), inheriting compatible settings, that reads the source as linked context on its first message. The source is untouched. config (same Provider only) sets live options such as model/reasoning; inherit_settings (default true) copies compatible settings. A stable operation id returns the same copy on retry and refuses a different request";
    "sessions.fork", true, "{session_id:string,body:{operation:string,message_id?:string,config?:object}}; creates a new session beside the source on the same Provider, subscription, Machines and project whose agent natively copies the conversation (Claude Code, Codex); message_id (an agent_message_chunk messageId ending a completed turn) forks through that answer. Whole forks need an idle source. The worktree starts from the source's HEAD plus uncommitted and untracked non-ignored files. Stable operation id; a retry returns the same destination. config sets live options (model, reasoning) on the copy before its first launch. For another agent or subscription use sessions.duplicate (a copy) or sessions.context_set (continue)";
    "sessions.placements", false, "{query?:{runtime_machine_id?:string,machine_id?:string}}; discovers compatible remote execution placements and Providers";
    "sessions.wake", true, "{session_id:string}; restores the worker without submitting a model turn";
    "sessions.reload_plan", false, "{session_id:string}";
    "sessions.reload", true, "{session_id:string,query?:{upgrade_provider:boolean}}; read reload_plan before applying";
    "sessions.update_policy", true, "{session_id:string,body:{enabled?:boolean,when_idle?:boolean}}";
    "sessions.hibernate", true, "{session_id:string}";
    "sessions.cache_protection", false, "{session_id:string}";
    "sessions.questions", false, "{session_id:string,query?:{before:number,limit:number}}";
    "sessions.question", false, "{session_id:string,page:string}; exact question-page sequence as a decimal string";
    "sessions.calls", false, "{session_id:string}";
    "sessions.call", false, "{session_id:string,call:string}";
    "sessions.call_cancel", true, "{session_id:string,call:string}; request cancellation of this exact managed call; observe terminal state";
    "sessions.calls_dismiss", true, "{session_id:string,body:{call_ids:string[],dismissed:boolean}}; hide at most 100 ended calls; false restores them; keeps results and admission receipts";
    "sessions.rename", true, "{session_id:string,title:string}";
    "sessions.pause", true, "{session_id:string,paused:boolean}";
    "queue.options", false, "{session_id:string}; delivery target, queue pause and versioned policy; does not wake a worker";
    "queue.policy_set", true, "{session_id:string,mode:'steer'|'tool_boundary'|'next_turn',revision:number}; CAS session preference, including empty or dormant sessions; affects future choices only";
    "queue.release_hold", true, "{session_id:string,operation:string}; explicitly release the matching interrupt hold only on an idle worker after inspecting transcript/receipt; never resends the replacement; preserves Pause";
    "sessions.configure", true, "{session_id:string,config_id:string,value:string|boolean}; configuration.read('session') discovers Provider choices";
    "sessions.cancel", true, "{session_id:string}; accepted means cancellation requested, not confirmed stopped";
    "sessions.cancel_subagent", true, "{session_id:string,subagent_session_id:string}; stop one ACP subagent (the sessionId of its cowboy_subagent row) while the turn continues; only while that row is cancellable; observe the row's state";
    "sessions.retry", true, "{session_id:string}; explicitly repeats the last failed prompt";
    "sessions.reset", true, "{session_id:string}; destructive: clears conversation context";
    "sessions.delete", true, "{session_id:string}; PERMANENT, skips Trash: accepted means close/delete requested; observe session absence before assuming deletion. Prefer sessions.trash";
    "sessions.trash", true, "{session_ids?:string[],folder_ids?:string[]}; move to Trash (recoverable until the owner's preferences.trash retention_days). A folder takes every session filed in it. All-or-nothing: a working session refuses the request. Observe trashed_at_ms on the sessions";
    "sessions.restore", true, "{session_ids?:string[],folder_ids?:string[]}; restore from Trash to the original folder, or its nearest live ancestor. A folder restores everything below it. Restored sessions stay asleep until opened or sent work";
    "sessions.purge", true, "{session_ids?:string[],folder_ids?:string[]}; PERMANENT deletion of items already in Trash; a folder takes its trashed contents";
    "queue.submit", true, "{session_id:string,text:string,content?:unknown[],delivery?:'next_turn'|'current_turn'|'next_checkpoint'|'interrupt'|'session_default',target?:{worker_epoch:string,turn_id:string},front?:boolean,force?:boolean}; omitted delivery is next_turn. Read queue.options for exact target and native controls. next_checkpoint consumes after foreground tools or at turn end. interrupt requires target, cancels that turn and sends once while preserving Pause/other rows. session_default resolves before self-steer checks. Observe pending/unknown receipts; never retry with a new operation";
    "queue.edit", true, "{session_id:string,id:string,text:string,content:unknown[]}; explicit complete replacement; preserve existing attachment blocks";
    "queue.delivery_receipt", false, "{session_id:string,operation:string}; observe an HTTP/CLI current-turn delivery without resending; MCP control.apply uses its existing control receipt";
    "queue.remove", true, "{session_id:string,id:string}";
    "queue.clear", true, "{session_id:string}";
    "queue.send", true, "{session_id:string,id:string}";
    "queue.force_send", true, "{session_id:string,id:string}; interrupts current work";
    "queue.to_draft", true, "{session_id:string,id:string}";
    "queue.reorder", true, "{session_id:string,order:string[]}";
    "drafts.add", true, "{session_id:string,text:string,content?:unknown[]}";
    "drafts.edit", true, "{session_id:string,id:string,text:string,content:unknown[]}; explicit complete replacement";
    "drafts.remove", true, "{session_id:string,id:string}";
    "drafts.move", true, "{session_id:string,id:string,to_session:string}; both sessions must be owned";
    "drafts.clear", true, "{session_id:string}";
    "drafts.activate", true, "{session_id:string,id:string}";
    "drafts.activate_all", true, "{session_id:string}";
    "drafts.reorder", true, "{session_id:string,order:string[]}";
    "drafts.schedule", true, "{session_id:string,id?:string,text?:string,content?:unknown[],fire_at_ms:number,delivery?:'back'|'front'}";
    "drafts.unschedule", true, "{session_id:string,id:string}";
    "usage.refresh", true, "{account?:string}; shared per-account cooldown and in-flight deduplication; Service collectors return immediately; named account IDs await their bounded native query; no account also refreshes your named accounts once, serially";
    "usage.reset", true, "{account:string,body:{confirm:'confirm',expected_credit_id:string}}; consumes a reset credit; explicit user intent required";
    "usage.reset_schedule", true, "{account:string,body:{fire_at_ms:number,confirm:'confirm'}}; schedules credit consumption";
    "usage.reset_cancel", true, "{account:string}; cancels only a scheduled reset";
    "usage.executor_set", true, "{account:string,body:{machine_id:string|null}}";
    "usage.activity", false, "{account:string,query?:{window?:string,model?:string,agent?:string,from_ms?:number,to_ms?:number}}; cached activity; model/agent are comma-separated filters";
    "usage.logs", false, "{}; select the required collector log";
    "diagnostics.logs", false, "{query?:{kind?:string,severity?:string,state?:string,agent?:string,window?:string,from_ms?:number,to_ms?:number,session?:string,cursor?:string,limit?:number}}; pass next_cursor to continue";
    "diagnostics.metrics", false, "{}; use selection to avoid full infrastructure metrics";
    "diagnostics.log", false, "{id:string}";
    "diagnostics.incidents", false, "{}";
    "documents.history", false, "{id:string}; metadata index; content only through history_snapshot";
    "documents.history_snapshot", false, "{id:string,revision:string}";
    "documents.undo_copy", true, "{session_id:string,cmid:string,body:{text:string,content:unknown[]}}; removes the draft copy only if unchanged";
    "sync.read", false, "{state:'title'|'order'|'workspace-order'|'folders'|`queue:${string}`|`mobile-review:${string}`}; only owned sessions/folders";
    "sync.apply", true, "{state:string,name:string,args:unknown}; existing typed UI sync mutation and permission checks; operation supplies mutation id";
}

pub fn find(name: &str) -> Option<&'static Operation> {
    OPERATIONS.iter().find(|op| op.name == name)
}

pub fn list(domain: Option<&str>) -> Value {
    if let Some(domain) = domain {
        return json!({"operations":OPERATIONS.iter().filter(|op| op.name.split('.').next()==Some(domain))
            .map(|op| json!({"name":op.name,"write":op.write})).collect::<Vec<_>>()});
    }
    let mut domains = std::collections::BTreeMap::<&str, usize>::new();
    for op in OPERATIONS {
        *domains
            .entry(op.name.split('.').next().unwrap())
            .or_default() += 1;
    }
    json!({"domains":domains,
        "next":"control.list(domain), then control.describe(name)",
        "exceptions":{"identity":"Interactive login, passkeys, credential export and granting authority stay with their identity owner.",
        "host":"Device-local config, binary deployment and host enrollment use their owning CLI; no host credential is inherited by a session.",
        "native":"Filesystem/code editing and native agent execution retain the runtime's native tools.",
        "streams":"UI subscriptions, telemetry ingestion and browser leases retain their protocol owners."}})
}

pub fn describe(name: &str) -> Option<Value> {
    let op = find(name)?;
    let mut value = json!({"name":op.name,"write":op.write,"input":op.input,
        "invoke":if op.write {"cowboy_execute({action:'control.apply',operation,arguments:{name,input}})"} else {"cowboy.control.read(name,input,{path?,offset?,limit?,chars?,describe?})"},
        "receipt":if op.write {Some("cowboy_receipt({domain:'control',operation}); accepted is submission, not completion. Pending/unknown is not safe to replay under a new id.")} else {None}});
    match name {
        "tools.defaults_set" | "tools.session_set" => value["types"] = json!(TOOLS_TYPES),
        "projects.request" => {
            value["types"] = json!(
                "type MachineWorkspace={id:string,display_name:string,canonical_path:string};"
            )
        }
        "sync.apply" => value["mutations"] = sync_mutations(),
        _ => {}
    }
    Some(value)
}

pub const SDK: &str = r#"
export interface Cowboy {
 control: {
  list(domain?:string):Promise<unknown>; // small catalog; no payloads or schemas
  describe(name:string):Promise<unknown>; // exact operation input, authority and receipt contract
  read(name:string,input?:object,selection?:{path?:string;offset?:number;limit?:number;chars?:number;describe?:boolean}):Promise<unknown>;
 };
}
// Writes: cowboy_execute({action:'control.apply',operation,arguments:{name,input}}).
// Reads cannot invoke writes. Same operation + identical input never submits twice.
// Receipts: cowboy_receipt({domain:'control',operation}). Pending/unknown needs observation,
// never an automatic new operation. Domain admission does not prove asynchronous completion.
// Prefer MCP for Cowboy state and control. CLI remains a supported fallback when the
// live MCP capability is unavailable; do not repeatedly reload help at the same revision.
"#;

const TOOLS_TYPES: &str = r#"
type CallTarget={agent:string,preset?:string,options?:Record<string,string>}; // preset/options mutually exclusive
type ReviewPreferences={claude:('opus'|'fable')[],gpt:('astra'|'sol')[]}; // nonempty, unique, preferred first; defaults opus→fable and astra→sol
type AgentTools={schema:1,matrix:{tools:boolean,recall:boolean},calls:{enabled:boolean,targets:CallTarget[],default:string,max_concurrent:number,max_per_session:number,review?:ReviewPreferences}};
type SessionTools={schema:1,matrix?:{tools?:boolean,recall?:boolean},calls?:{enabled?:boolean,targets?:CallTarget[],default?:string,review?:Partial<ReviewPreferences>}};
// Review only; each omitted family inherits the agent-kind Settings default.
// Never widen calls permissions when changing a preference. Read the current document and preserve unrelated fields.
"#;

fn sync_mutations() -> Value {
    json!({
        "title":{"rename":"{session_id:string,title:string}"},
        "order":{"reorder":"{order:string[]}"},
        "workspace-order":{"reorder":"{order:string[]}; session:<id> / draft:<id>, only the caller's ordering"},
        "folders":{
            "create":"{id:string,name:string,parent?:string|null,project?:string|null,defaults?:Record<string,string|null>}",
            "rename":"{id:string,name:string}", "move":"{id:string,parent:string|null}",
            "reorder":"{parent?:string|null,order:string[]}", "bind":"{id:string,project:string|null}",
            "place":"{session_ids:string[],folder:string|null}", "remove":"{id:string}; keeps child folders and sessions",
            "defaults":"{id:string,values:Record<string,string|null>,inherit?:string[]}; keys from folders.defaults_set"
        },
        "mobile-review:<owned-session>":{
            "setDirectory":"{directory:string|null}|{inherit:true}; prefer preferences.update for revision-checked defaults",
            "setRemoteReview":"{binding:null|{plugin_id:string,view:string,host:string,owner:string,repository:string,repository_id:string,number:string}}",
            "selectRemoteReview":"{selected:boolean}",
            "open":"{path:string}", "close":"{path:string}", "activate":"{path:string|null}",
            "reorder":"{paths:string[]}", "setPinned":"{path:string,pinned:boolean}",
            "setMode":"{mode:'files'|'git'}", "markReviewed":"{key:string,revision?:string|null}",
            "setPosition":"{path:string,line:number,revision?:string|null}"
        }
    })
}
