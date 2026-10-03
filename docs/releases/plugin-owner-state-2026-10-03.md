# Bounded, closed Cowboy component owner decisions — October 3

Recovery selection depends on the component journal and last successful receipt
to bind the transaction and accepted target. Those reads previously used an
unbounded `ReadFile` and ordinary `json.Unmarshal`, accepting duplicate/unknown
fields and case aliases. A special file could also block the owner before its
recovery guard ran. Existing recovery-selection archive reads were unbounded.

The Columbus owner now opens regular files with `O_NOFOLLOW` and `O_NONBLOCK`,
checks the opened descriptor's type/size and caps bytes read even if the file
grows after `Stat`. Component journal, success receipt and an existing archived
selection are limited to 64 KiB each. The reader floor retains its 8 KiB limit
and shares the bounded file reader. Missing files keep their existing meaning;
symlinks, directories, FIFOs and oversized files refuse. Refusal does not remove,
rewrite, repair or truncate owner evidence.

The journal and success receipt require one JSON object with known, canonical
field names, no duplicate keys or trailing values. Reader declarations and floor
state use the same exact-field decoder, closing `Schema`/`ReaderSchema` aliases
that Go otherwise accepts. Existing semantic identity, ancestry, generation and
reader guards still apply. This does not introduce a new state schema, change
the writer's output format, admit cross-generation recovery, fence independently
run old tools or enable Session deletion writing.

## Source and checks

Published Columbus source is `3520f6821760ad97461290e720d0b517c145dc05`. It comes from the isolated
owner task worktree, integrates fresh remote main and the already active
`e2799fe62a46f3d3f4443929327886e86ad938bf` host source. Pinned-shell `just verify`,
all Machine Go packages with `go test -race ./...`, and `go vet ./...` passed.

Four test groups cover journal/receipt malformed decisions, file types and
nonblocking FIFO refusal, reader declaration case aliases and oversized archived
decisions. Duplicate/unknown/case/oversized JSON fixtures first demonstrate
acceptance by the former decoder; the new owner must refuse before ancestry,
profile resolution or recovery-root effects. Trailing and null objects also
refuse. Tests verify journal, current receipt, floor and archived evidence remain
unchanged on refusal. Existing canonical schema-1, recovery-selection replay,
Controller/Web/Machine recovery and reader-floor fixtures continue to pass.

These are owner I/O and parser fixtures, not a power-loss experiment or an
intentionally failed production Machine recovery. No production record is seeded.

## Owner activation

The clean committed Hawk closure was built and activated through the owning
`machines/justfile` transaction. Host transaction
`1791032319712918869-3520f6821760` succeeded, published, at
`2026-10-03T20:58:42+08:00` from
`/nix/store/jrk2dd33a0qij7dagxbx00cm27q2nbis-nixos-system-hawk-26.05.20260731.5b4f72e`.
Built unit comparison and the host receipt both list only `mandb.service` as
changed, no changed user units and no explicit restarts. All required health
checks passed and no new failed units were recorded.

The installed activator is
`/nix/store/q9w4vwg145l3jmrl94myfhvznl9n3f0c-columbus-machine-activate-1da44eb/bin/cowboy-release-activate`,
SHA-256 `d778511ca111fe91d0266e102aa8d3da65d83e0e0d830fbc4568a5a00f7ce71e`.
Its pre-dispatch Machine repair check read the unchanged canonical production
floor and refused the undeclared predecessor with exit 1 and
`durable Session deletion reader floor refuses legacy Machine`. Profile,
component receipt and floor bytes were unchanged, and no component journal was
created. No malformed production record or failed recovery was seeded.

Bounded samples at `2026-10-03T12:57:30.802Z` and
`2026-10-03T12:59:16.957Z` retained all 13 original ACP worker and four
execution keeper PIDs. Machine PID `1928418` and Controller PID `486493` were
unchanged, as were the Machine component receipt, root-owned floor bytes and
Web profile. HTTPS health/version/SPA/SW/deployment-health returned 200;
HTML/SW retained `no-store`, the separate SPA version was unchanged, and Machine
reported connected/online with `worker-6ede7a91cc8b8b3402d4`. These samples do
not establish full generation rollout or native resume.

Production deletion state remained only `.lock`; the unchanged Machine writer
remains disabled. No Cowboy component, portable device or iOS release was
activated. The [machine-readable receipt and observations](../experiments/plugin-owner-state-2026-10-03.json)
retain exact activator hashes, host receipt, component samples and the live floor
refusal. Cross-generation recovery, portable reader admission, independent old
tool authority and production writer release acceptance remain open.
