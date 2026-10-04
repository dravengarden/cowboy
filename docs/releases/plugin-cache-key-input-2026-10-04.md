# Bounded cached-host publisher key input

Ordinary portable cached-host startup now uses the same publisher-key admission
as explicit floor/anchor recovery: a regular file, no final symlink following,
nonblocking open, at most 16 KiB, and valid UTF-8 before signature checking.
Previously this path used unbounded `read_to_string`, which followed links and
could wait forever on a FIFO before producing the startup decision. Empty
selection without a floor still needs no publisher key and creates no state.

This intentionally refuses publisher-key symlinks, including a symlink to valid
key bytes. Configure the existing recovery-compatible regular public-key file.
It does not rotate a key, enable deletion writing, change a signed transcript,
restrict artifact size or fence administrators and same-user concurrent writers.

Source fixtures cover raw and archive caches with key link, FIFO, oversized and
invalid UTF-8 inputs. Actual immutable-launcher acceptance extends the existing
24 startup cases to 32. Each refusal must leave selected/fallback execution
markers absent and selection pointers unchanged. A pinned timeout helper bounds
the disposable process group, so a hung reader cannot be mistaken for a normal
refusal. With `COWBOY_TEST_PORTABLE_PRE_KEY_RELEASE`, preceding exact native
controls demonstrate valid-key symlink admission and FIFO timeout. The test
exports its complete observations through `COWBOY_TEST_CACHE_KEY_RECEIPT`.
Source gates, exact binary identities and production activation follow below.


Source `5a84eae060014ea10d84a793bb5c2846d5b30776` passes format, all-feature Clippy and the explicitly
packaged standalone Machine-host Clippy targets. The complete library gates pass
1,845 all-feature tests (53 ignores) and 526 Machine-host tests (15 ignores) at
four test threads. The first all-feature run passed 1,844 and failed one unmodified
installer fixture with `Text file busy` during a bootstrap probe; the complete
rerun passed. No retry or installer behavior was changed.

Artifact `/nix/store/8gygd38dk4qh1ln771idi1ph2nhh875x-cowboy-machine-release` retains the six independently accepted companion
paths and digests and worker source `b97c2724bea23834944ded8af98e2de6729f4256`,
generation `worker-748825b42b4302fe26ca`. Its actual native launcher passes all
32 cases: four healthy launches exit zero, 28 refusals exit one, no timeout or
fallback execution. Preceding native `/nix/store/sd59vv8dh9m5d8mgh7zj7bgliy78ckq6-cowboy-machine-release` admits the
valid key symlink and times out on FIFO for both artifact formats; only disposable
fixture process groups are terminated. All four 240 MiB payload diagnostics under
192 MiB address-space limits still pass, with both tamper refusals and unchanged
floors. Production limits and public-key files were not modified.

Root owner transaction `1791112873182594468-5a84eae06001` started at 2026-10-04T11:21:13.182594468Z and
committed at 2026-10-04T11:21:22.230333259Z. It reports succeeded/committed, published true
and recovered false; the independent activator reports success. Resident Machine
PID changed from 2615815 to 2790245; all
13 worker and 5 keeper IDs, PIDs and states remain identical. Controller PID
2336663 and its root receipt remain identical, along with
the Web profile. All five health/version/SPA/worker-presence HTTP checks return
200. Deletion startup reports zero deleted Sessions and writer false; the portable
namespace contains only `.lock`, and the reader floor SHA-256 remains
`26910e8cf5add044da3bf74ab2ed56161d2321113d9662e27952e16cc25ae017`. Noninteractive sudo is available, with unchanged sudoers
and installed root activator digests. The
[full production receipt](../experiments/plugin-cache-key-input-2026-10-04.json)
retains exact native/installer identities, controls and before/after observations.
