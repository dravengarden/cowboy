# Product CLI

Everything the Web UI does is also reachable from `cowboy` on any computer that
has run `cowboy login` once. The commands use the same authenticated HTTP routes
and WebSocket commands as the Web UI, so Service permissions, session visibility,
Provider compatibility gates and refusals are identical.

## Login

```bash
cowboy login https://cowboy.example
```

The first login asks a human to approve this computer's public key in the
browser (or through the Cardea broker, see `cardea-device-login.md`). After that
the device stays signed in for one year from approval: 10-minute
sender-constrained access tokens refresh silently from a rotating refresh token
whose deadline rotation never extends. Account → Authorized devices revokes it at
once. Plugin installation, session lifecycle and the generic commands below need
no further human approval; only Machine registration, Provider account login and
a new device login involve a person.

Commands take `--origin` (or `COWBOY_ORIGIN`) and default to the only origin this
computer is logged in to.

## Sessions

```bash
cowboy session list [--outdated] [--json]
cowboy session info ID
cowboy session update ID... | --all [--force | --when-idle] [--auto on|off]
cowboy session reload ID [--force]
cowboy session hibernate ID
cowboy session prompt ID "text"
cowboy session cancel ID
cowboy session clear ID --yes
cowboy session rename ID "title"
cowboy session delete ID --yes
cowboy session new PROVIDER [--cwd DIR]
```

`update` moves sessions to their Machine's newest installed Provider release
through the confirmed Reload path. A busy session is refused unless `--force`,
which cancels the running turn first. `--when-idle` queues one update after the
current work; `--auto on` keeps a session on every future release once idle.
A session whose prompted context has no saved native session cannot change
version; the command reports that reason.

## Any route or command

```bash
cowboy api GET /api/sessions/ID/calls
cowboy api PUT /api/sessions/ID/reload --data '{"enabled":true}'
cowboy send '{"type":"set_paused","session_id":"ID","paused":true}'
```

`api` signs any HTTP route the Web UI calls and prints the response body.
`send` delivers one WebSocket command (the `Inbound` protocol in `src/core.rs`)
and fails if Cowboy refuses it. A Web UI capability without a dedicated
subcommand is therefore never out of reach; frequently used ones get a
subcommand like `session`.

## On the Controller host

`cowboy operator` uses the Controller's private host endpoint instead of a
device login, so the Service account can keep everything current unattended:

```bash
cowboy operator converge --apply                     # Plugins on every Machine
cowboy operator sessions [--outdated] [--json]
cowboy operator session-update --all [--except ID]... [--when-idle | --force]
cowboy operator session-update ID...
```

`session-update` reloads idle sessions on the newest installed release,
re-pins exited ones without starting them, and refuses busy ones unless
`--when-idle` (update right after the running turn) or `--force` (cancel the
turn, then update). The same compatibility gates as the Web UI's Reload apply.
