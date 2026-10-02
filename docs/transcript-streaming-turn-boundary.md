# Streaming carets belong to the current turn

Sending a new prompt can make the session Busy before the prompt echo reaches
the transcript. Optimistic text/image bubbles are rendered independently of the
canonical render items. A completed assistant reply can therefore still be the
last canonical item while a new prompt says Sending.

The previous predicate used only Busy and the last item's role. That revived a
blinking caret inside the completed reply, making it look editable or unfinished.
Successful `turn_end` events are intentionally absent from rendered items, so
looking only at those rows cannot distinguish these turns.

The streaming predicate now checks canonical event boundaries against the row's
stable first-envelope sequence. A turn end, lifecycle transition or new user
prompt after the row prevents its caret from being reactivated. Current reply
and thought rows retain their streaming marker. Canonical boundaries also win
when the presented timeline is delayed during scroll/drawer catch-up. Scan only
events at or after the row, rather than older history.

Sending and waiting stay on the new prompt/activity row. No editable controls,
focus behavior, composer extensions or gesture layers change.

Regression coverage folds real canonical events and checks completion → Busy
before echo, text/image echo, first new reply, idle/disconnected display,
cancellation/error completion, thought rows and delayed presentation. Provider
independent: Claude and Codex use the same transcript predicate.
