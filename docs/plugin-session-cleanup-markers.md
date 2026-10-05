# Bounded Cargo cleanup markers

The [October 5 Hawk release](releases/plugin-session-cleanup-markers-2026-10-05.md)
records the activated artifact and bounded continuity evidence.

Deleted-session cleanup recognizes a Cargo `target` only when both
`.rustc_info.json` and `CACHEDIR.TAG` are regular files opened with no-follow and
nonblocking flags. Both marker names resolve relative to the same opened target
directory. The JSON marker is checked for file type without reading its contents.
The tag must contain the existing signature and valid UTF-8 within 8192 bytes.
Metadata rejects an already oversized tag; a limited read and post-read byte check
also bound a file that grows during reading.

Previously a tag FIFO could leave the cleanup thread waiting for a writer while
it retained the Session lifecycle gate. Marker symlinks were followed and tag
content had no size bound. Symbolically linked, nonregular, unreadable, oversized or invalid
markers now leave the target ineligible for removal. Ordinary marked targets
remain eligible, including a tag at the exact byte limit. No journal format,
worker protocol, SDK or dependency changes are required.

Regression cases run complete cleanup on FIFO markers without any writer and
require completion using a bounded test wait. They verify artifact preservation
for links on either marker, an oversized tag containing a valid signature,
invalid UTF-8 and a directory marker. Existing cleanup fixtures verify normal
target reclamation. The exact-size boundary remains accepted.

This bounds marker content and prevents FIFO waits; it is not a general wall-clock
deadline for filesystem I/O. Directory-relative marker probes do not establish
continuous nested-directory ownership or protect later pathname-based recursive
removal from every independent nested replacement. The original Session root
handle and process-exit proof retain their separate contracts.
