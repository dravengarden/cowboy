// The two session-lifecycle actions the composer exposes as one-tap buttons.
// They dispatch DIFFERENTLY, because the two concepts live on opposite sides of
// the ACP boundary:
//
//   • "compact" is an AGENT operation — the agent summarises its own context.
//     Claude/Codex/Gemini/Grok expose it as a real slash-command, so we resolve the
//     concrete `/command` and send it as a prompt (kind: "slash"). The spelling
//     differs per CLI, so we resolve it from (1) what the agent advertises over
//     ACP (`available_commands_update`, authoritative — matched by an alias set),
//     then (2) a per-provider default. No match ⇒ null ⇒ the button hides.
//
//   • "clear" is a CLIENT operation — dropping the conversation and starting
//     fresh. No agent exposes a `clear` command over ACP (verified: the Claude
//     adapter advertises `compact` but no clear/new/reset), because clearing is
//     the client's job. So clear is NOT a slash-command: it's a cowboy session
//     RESET (kind: "reset") that respawns the agent with a fresh session/new.
//     It works for every agent, so it's always available.
import type { ProviderUiManifest } from "../../packages/provider-ui-sdk/src/index.ts";
import type { AcpUpdate, AvailableCommand, Envelope } from "./protocol";
import { currentProviderEntry } from "./providerCatalogRegistry";

export function latestAvailableCommands(timeline: readonly Envelope[]): AvailableCommand[] {
  for (let index = timeline.length - 1; index >= 0; index -= 1) {
    const envelope = timeline[index];
    if (envelope?.kind !== "update") continue;
    const update = envelope.update as AcpUpdate;
    if (
      update.sessionUpdate === "available_commands_update" &&
      Array.isArray(update.availableCommands)
    ) return update.availableCommands;
  }
  return [];
}

export type SessionActionId = "compact" | "clear";

export interface SessionAction {
  id: SessionActionId;
  /** "slash" ⇒ send `command` as a prompt; "reset" ⇒ a cowboy session reset. */
  kind: "slash" | "reset";
  /** The `/command` to send (leading slash). Present only when kind === "slash". */
  command?: string;
  /** Button + confirm-dialog title. */
  label: string;
  /** One-line consequence, shown in the confirm dialog. */
  detail: string;
  /** Confirm button verb + colour intent ("clear" is destructive). */
  destructive: boolean;
}

type CompactionContract = NonNullable<
  ProviderUiManifest["host"]["conversation_compaction"]
>;

export function resolveCompactionAction(
  contract: CompactionContract | undefined,
  available: readonly AvailableCommand[],
): SessionAction | null {
  if (!contract) return null;
  const aliases = new Set(contract.aliases.map((alias) => alias.toLocaleLowerCase()));
  const advertised = available.find((command) => aliases.has(command.name.toLocaleLowerCase()));
  const name = advertised?.name ?? contract.fallback_command;
  return {
    id: "compact",
    kind: "slash",
    command: `/${name}`,
    label: "Compact conversation",
    detail:
      "Summarise the conversation so far into a shorter context. The agent keeps working, with the condensed history in place of the full transcript.",
    destructive: false,
  };
}

// Clear is a client-side reset — always available, no agent command involved.
const CLEAR_ACTION: SessionAction = {
  id: "clear",
  kind: "reset",
  label: "Clear conversation",
  detail:
    "Start the agent on a fresh context, discarding the conversation so far — it won't remember anything above. The transcript stays on screen (a divider marks the cut) so you keep the record. This can't be undone.",
  destructive: true,
};

/** Resolve a composer session-action for this session's agent, or `null` when
 *  it doesn't apply (only compact can be null — clear is always available). */
export function resolveSessionAction(
  id: SessionActionId,
  provider: string,
  available: readonly AvailableCommand[],
  providerVersion?: string,
  providerDigest?: string,
): SessionAction | null {
  if (id === "clear") return CLEAR_ACTION;
  const contract = currentProviderEntry(provider, providerVersion, providerDigest)
    ?.manifest.host.conversation_compaction;
  return resolveCompactionAction(contract, available);
}
