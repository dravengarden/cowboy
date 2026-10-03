# Hawk root-identity adoption observed October 3

The September root-object and accepted-configuration identity records described
an unactivated Machine candidate. Hawk's later maintenance transaction now
includes both implementations. This reconciles the completion ledger with the
actual component receipt; it does not change the historical candidate evidence.

- Machine source: `89bf4b720fb7c0048e71b193ab6b0b6e6bf7b134`.
- Release: `/nix/store/1qbhcdgwxyc18fhm95phqnqda0hjg86s-cowboy-machine-release`.
- Worker generation: `worker-2801f50d44994e96b2b4`; source protocol 24.
- Transaction: `1790908850252980862-89bf4b720fb7`.
- Receipt: `/var/lib/hawk-component-deployments/cowboy-machine/current.json`,
  observed October 3; `outcome: succeeded`, `phase: committed`,
  `maintenance: true`, `published: true`.
- Recorded at `2026-10-02T02:40:57.990841244Z`.
- `git merge-base --is-ancestor bc385b9d 89bf4b72` succeeds: the activated source
  contains the accepted-configuration retirement fix and preceding root fence.

The receipt establishes production adoption. The original source and isolated
connected checks remain the implementation evidence. No additional production
configuration ABA was injected, and this does not close Session-worktree
identity, security-domain ownership, state leases or native recovery exits.
