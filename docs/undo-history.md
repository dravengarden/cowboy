# Undo history

Every change the Web client can reverse is announced once and then kept in
**Undo history**, so a notice can be short without making undo hard to reach.

## Notices

Every transient notice (`DocumentNotifications` and the app error toast) shows
for `UNDO_NOTICE_MS` = 3 s. Hovering pauses it; an Undo in flight holds it until
the outcome is known. An undoable notice offers **Undo** and an **Undo history**
button. Undoable changes are recorded only through `recordUndo()`
(`web/src/undoActions.ts`); `documentNotice()` no longer accepts a closure.

## Entries and states

An entry is a message plus serializable steps (`web/src/undoHistory.ts`):
Trash restore of sessions/folders, Draft restore or re-trash at a revision,
removing an unchanged copied unsent row, copying text back to a Session, and
restoring cleared Calls. Steps run in order once.

| State | Meaning |
| --- | --- |
| Can undo | Steps are available and the target still allows it |
| Undoing… | Steps are running |
| Undone | Completed, with the time it was undone |
| Unavailable | Cannot run, with the reason |

Unavailable is shown when an undo failed (the Controller's reason, such as a
copied draft that was edited or sent; earlier steps are not rolled back, so it
is never retried blindly), when a Trash item was already restored or deleted
permanently (derived live from the Trash view once the Controller lists are
loaded), when the page closed during an undo, or when steps over 256 KiB (large
attachments) stayed in the memory of the page that made the change.

## Retention

History is device-local `localStorage` (`cowboy.undoHistory.v1`), filtered by
the signed-in principal, shared across tabs through `storage` events, and
removed on product sign-out because entries can quote Draft text.

The TTL is 24 hours with at most 100 entries. Undo is for recent mistakes: a day
covers "deleted last night, noticed this morning" while keeping the list short.
Long-term recovery belongs to Trash, whose retention is separately configurable.
Expired entries are pruned on load, on every new entry, every 10 minutes and
when the history opens.

## Entry points

- The history button on an undoable notice.
- **Undo history** in the Trash actions sheet (Mobile and Desktop).
- Desktop `␣I` and the command palette ("Undo history…").
