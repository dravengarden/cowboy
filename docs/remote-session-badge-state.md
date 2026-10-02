# Remote session badges

The session list shows the AI runtime and the files/commands Machine as
`OVH → Hawk`. The persisted execution binding supplies the route; directory
names and session titles never supply a target.

The badge must retain that route if execution validation fails. A runtime cwd
disagreement, unsupported executor protocol, or incomplete workspace makes the
executor unavailable; it does not turn the session into a local session. Show a
warning icon and explain the unavailable environment in the accessible label
and existing details sheet. Execution launch validation remains unchanged.

The presentation accepts route metadata only from schema 1 bindings whose
runtime Machine matches the session. Missing, null, foreign-runtime and unknown
schema bindings do not establish a remote target.

On 2026-10-02, read-only inspection of the two reported OVH sessions found
explicit Hawk bindings. Both already project `OVH → Hawk` with current source.
The reported plain `ovh` screenshot therefore does not prove the executor-state
failure or a stale browser bundle. This change closes the demonstrated
unavailable-state presentation gap and advances the service-worker version so
installed clients can adopt it through their configured update policy.

Validation: 1,989 Web tests, TypeScript and Deno checks, Oxlint, component
consistency and the isolated Firefox project-placement browser suite. Browser
coverage includes target retention with a warning and touch/keyboard activation
without selecting the session. A physical client was not inspected.
