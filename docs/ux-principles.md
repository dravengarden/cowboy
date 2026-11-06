# Cowboy core UX principles

Status: core product contract. Applies to every Cowboy surface. Read this
before proposing or changing product interaction. The goal is efficient work:
reduce reach, repeated taps, context switching, waiting, and mistakes while
keeping the work itself visible.

## Efficiency is the deciding criterion

Evaluate a design with concrete tasks: open the next change, mark it reviewed,
find a file, switch a worktree, return to the Agent. Prefer fewer deliberate
actions and a stable location for each recurring action. Visual minimalism
does not justify hiding a frequent operation, and a crowded toolbar does not
justify reducing touch targets. Disclose secondary tools with meaningful
labels, without adding a second navigation hierarchy.

## Mobile: read above, operate below

- Keep frequent navigation, search, filters, context selection and reading
  actions in a persistent bottom workbench, within comfortable thumb reach.
  Do not require a top-of-screen reach or a scroll to recover those controls.
- Let the upper area present the work. A compact, passive file/branch label
  can orient the reader; stacked headings, tabs and toolbar rows cannot consume
  the start of every reading surface. Put expanded metadata in a bottom sheet.
- Group related bottom controls into compact rows. Let short view labels use
  their natural width, with comparison or filter controls beside them. Avoid
  stacked full-width segmented bars, oversized capsules and repeated dividers.
  Reduce decoration and wasted space while preserving 44 px touch targets.
- Put next/previous, review marks, view selection and search directly in their
  active view. Secondary actions may contain refresh, integrations and settings.
  Useful facts remain visible in the reader's context dock or information rail;
  follow the shared reader-dock contract in `web/AGENTS.md` for cover sheets.
- Put bottom chrome in the layout so the final content row remains reachable.
  Respect all safe-area edges, retain at least 44 CSS px for frequent touch
  targets, and wrap controls rather than clipping them or relying on a
  horizontal swipe at a system-gesture edge. File tabs above the action row may
  scroll; the action row itself must not.
- Preserve reading space in short landscape viewports by sharing horizontal
  space between action groups. Keep next/previous and review marks direct;
  measure the open document as well as the overview. Agent and Review share
  the pager's established bottom navigation baseline rather than independently
  changing its home-indicator spacing.
- Search stays above the software keyboard. Do not automatically focus an
  input merely to open navigation. Preserve the existing viewport/keyboard
  owner instead of adding another viewport tracker for a feature.
- Use paint-only selected states and the existing sheet/drawer primitives.
  Reading, taps and swipes must stay responsive with real code on screen.
  Follow the [mobile compositor contract](mobile-spatial-presentation.md).
- Use the shared theme-aware reader edges for vertical reading surfaces.
  Full-screen content can pass beneath a soft safe-area gradient; local scroll
  regions fade only where more content remains. Short content and boundary
  items remain clear. Keep focused input, sticky actions and nested dialogs
  unobscured, and preserve native editor and image-pan behaviour.

## Desktop: keyboard and available space

Desktop keeps useful actions visible and uses the available pane space. A
phone's overflow menu is not a reason to hide Desktop controls. Reuse native
focus, Vim motions, the command system and visible contextual hints; do not
add a second shortcut grammar. See the
[Desktop efficiency contract](desktop-efficiency-redesign.md) and
[focus/keyboard policy](../web/src/desktop/FOCUS.md). Touch tablets still need
reachable, adequately sized controls even when their screens are wide.

## State and feedback are part of efficiency

Keep context identity visible. Every action operates on the content the user
is actually reading. Preserve query, comparison, scroll, expanded directories
and review progress through ordinary navigation. Moving controls must not
duplicate their owners, trigger an operation during layout changes, or change
Agent context merely because another worktree is being inspected. Show pending,
failed and confirmed results accurately; never turn an uncertain write into
an apparent success to save a tap.

## Acceptance

Exercise complete tasks on the rendered product, with its real theme, in
light and dark modes. Check phone widths down to 320 CSS px, short/landscape
viewports, keyboard-open search, long paths, empty/loading/error states and
Desktop panes. Verify reachable controls, unobscured final rows, stable context
and direct repeat actions. Include screenshots and geometry when requesting a
visual review. Browser emulation verifies layout; physical iOS keyboard and
swipe behaviour requires device evidence and must be reported separately.
