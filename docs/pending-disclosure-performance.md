# Pending panel disclosure during live updates

Date: 2026-09-15. Baseline: `6a1eff6b`. Web release: `cowboy-v1689`.

## Cause and change

The AppBar and Composer attachment callbacks in `web/src/App.tsx` were inline
functions. A session-list update renders App again and creates new functions.
React calls the previous refs with `null`, then the new refs with the existing
DOM nodes. The elements themselves are not remounted.

`useFloatingComposerGeometry.observe()` treats this as a real detach/attach:
it unobserves the element, removes disclosure listeners, clears the active
transition set, measures with missing elements, and publishes temporary zero or
navbar-only heights. Reattachment then restores the actual dimensions. These
read/write cycles force extra layout and discard the hold that protects
Transcript clearance during disclosure. Quiet panels do not encounter the same
repeated work, which explains the dependence on concurrent UI updates.

Both callback refs are now stable. Their dependencies still include the surface
and navbar mode, so actual layout-mode changes update the drawer followers.
Editor state, native scrolling, attachments, and disclosure timing are unchanged.

## Isolated comparison

The fixture bundled the actual PendingPanel, MessagePreview, theme, and geometry
hook with the repository's pinned production Vite builder. It used three drafts
(two text-only, one inline image), an in-flow transcript with 150 static rows,
and an unrelated parent render every 80 ms. Only callback identity differed
between the two runs. No production account or session was used.

Ten alternating expand/collapse operations ran in Safari on a fresh iPad Pro
11-inch Simulator, iOS 26.5. Each sampled 700 ms after activation. Instrumentation
counted geometry reads, writes to the three geometry CSS variables, and
CodeMirror measurement calls.

| Measurement across ten disclosures | Inline callbacks | Stable callbacks |
| --- | ---: | ---: |
| Geometry variable writes | 2,279 | 421 |
| Zero/navbar-only stack-height writes | 216 | 0 |
| Synchronous `offsetHeight` reads | 1,101 | 395 |
| Time inside those reads | 400 ms | 85 ms |

CodeMirror only measured the previews on the first expansion; it was not
recreated on subsequent folds. That rules out preview remounts as the cause in
this reproduction. Earlier quiet fixtures also ran in Firefox and iPad Safari.

These counts establish the lifecycle regression and reduced layout work. They
are not a production FPS guarantee: Simulator frame intervals still varied,
and physical iPad scrolling, keyboard/IME, and compositor behavior were not
accepted by this fixture. The physical image-adjacent caret issue remains open.
Raw samples and the temporary runner are retained under
`/tmp/cowboy-disclosure.uXRmDn/` on the diagnostic host.
