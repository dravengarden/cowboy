# Private native input budgets

Zed Plugin/adapter `1.13.3` selects private server `1.0.1`, on the unchanged
upstream source and dependency pins. This is a candidate, not an installed
generation or permission to enable production navigation. Its finite change
bounds individual native inputs **before** expensive allocation. It does not
complete the [navigation acquisition budget](plugin-owned-navigation.md).

The [candidate acceptance](releases/native-input-bounds-candidate-2026-09-18.md)
records exact artifact hashes, the complete source gate, native filesystem/LSP
tests, real pair and signed lifecycle gates, eighteen connected checks and
twenty-four browser ownership regressions. No production component was activated.

## Actual input boundaries

The pinned upstream LSP stdout reader previously used `read_until` on an
unbounded header line and resized a vector directly from `Content-Length`.
The private distribution now limits the complete header to 8 KiB, including
its terminator, while reading. It validates a single case-insensitive decimal
Content-Length in `1..=2 MiB` before resizing or reading the message body.
Missing, duplicate (even equal), signed, overflowing and malformed lengths
refuse. Error messages contain no header values. The existing finite incoming
queue/backpressure remains in place; this is not a total process RSS bound.
An over-budget input ends that LSP reader. No truncation, resynchronization,
retry, replacement language server or invented navigation result is added.

The private native text loader previously checked a pathname's metadata against
an upstream 6 GiB limit, then read an unbounded descriptor into memory. All
private native text opens now use the existing source-owned bounded descriptor
reader: at most 4 MiB of raw bytes plus one overflow sentinel. On Linux it
refuses symlinks in every component, nonregular files and unsupported `openat2`
without a fallback. A growth race cannot bypass the actual read limit. Encoding
detection remains upstream-owned; decoded UTF-8 must also fit 4 MiB **before**
constructing a native text buffer. Temporary decoding storage may exceed the
raw byte count by a bounded encoding expansion; this is not an exact heap cap.
Ordinary native source opens and navigation targets use this same loader.

These changes neither reload an existing buffer nor write a source file.
Existing content/epoch checks and Unknown acquisition fences are unchanged.
In the `1.13.3` candidate, an upstream LSP conversion error may still be omitted
from an aggregate query result: these input checks alone do **not** establish
typed whole-query refusal or release uncertain target ownership. The separate
`1.14.0` [whole-query route](plugin-native-navigation-budgets.md) adds the former,
not the latter.

## Verification

The source-owned Nix native build includes `project`, `fs` and `lsp` tests.
Streaming tests cover exact/over-limit headers, unterminated lines, missing and
duplicate lengths, malformed headers, integer overflow and redacted errors.
The actual stdout dispatcher must reject an oversized declared body without
attempting to read it. Filesystem tests retain no-symlink/nonregular coverage
and prove that a growing input consumes only one extra sentinel byte.

The immutable native-process gate additionally opens an exact 4 MiB source,
rejects one extra raw byte, rejects bounded UTF-16 input that would produce
over-budget UTF-8 text, verifies unchanged files and probes the surviving
native process. The full native pair, signed disposable installation and
connected Code gates remain required. Synthetic language answers and fixture
cleanup are not production native-generation, device or restoration acceptance.

## Follow-up and still open

The `1.14.0` [candidate record](releases/native-navigation-budgets-candidate-2026-09-18.md)
adds all-server location/target budgets, original-worktree-only acquisition and
typed whole-query refusal. It bounds registered-server enumeration, admits one
private query handler at a time and limits observation deadlines. It does not
turn those bounds into global native quiescence or recovery.

- Bound aggregate live buffers/history and background effects, including loads
  that outlive cancellation or an observation deadline. No whole-process memory
  or worktree-scanning limit follows from the finite per-query bounds.
- Accept the exact signed native pair through the intended deployed consumer
  and supported devices, with separate Machine maintenance and installation.
- Independently authorized recovery of an uncertain acquisition; local close
  enqueue and process restart are not restored native state.

Production navigation admission must stay closed until those exits are accepted.

The `1.15.1` [reload-input candidate](plugin-native-reload-bounds.md) additionally
bounds the existing LocalFile reload reader and decoded text before diff/CRDT
mutation, and clears completed local reload tasks on failure. It does not add
automatic reload, read fallback, global history limits or recovery.
