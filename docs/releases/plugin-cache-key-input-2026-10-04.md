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
