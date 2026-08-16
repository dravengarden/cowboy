import { assertEquals } from "jsr:@std/assert";

const managementSource = await Deno.readTextFile(
  new URL("./ProviderManagement.tsx", import.meta.url),
);
const surfaceSource = await Deno.readTextFile(
  new URL("./ProviderSurface.tsx", import.meta.url),
);
const transcriptPresentationSource = await Deno.readTextFile(
  new URL("./ProviderTranscript.tsx", import.meta.url),
);
const grokProviderSource = await Deno.readTextFile(
  new URL("../../providers/grok/provider.json", import.meta.url),
);

Deno.test("Provider lifecycle cards stay collapsed behind a compact summary", () => {
  assertEquals(managementSource.includes("const [detailsOpen"), true);
  assertEquals(managementSource.includes("hidden={!detailsOpen}"), true);
  assertEquals(
    managementSource.includes('display: detailsOpen ? "grid" : "none"'),
    true,
  );
  assertEquals(managementSource.includes("borderTop:"), false);
});

Deno.test("incompatible Provider lifecycle actions are omitted instead of looking actionable", () => {
  assertEquals(surfaceSource.includes("if (blocked) return null;"), true);
  assertEquals(surfaceSource.includes("disabled={busy || blocked"), false);
});

Deno.test("Machine compatibility renders once in the Cowboy warning shell", () => {
  assertEquals(
    managementSource.includes("const surfaceError = error || releaseError;"),
    true,
  );
  assertEquals(managementSource.includes("{error || !releaseReady"), false);
  assertEquals(
    managementSource.includes("error || compatibilityDetail"),
    false,
  );
});

Deno.test("Provider management cards keep geometry in the Cowboy component library", () => {
  assertEquals(
    managementSource.includes("data-provider-management-identity"),
    true,
  );
  assertEquals(
    managementSource.includes(
      'gridTemplateColumns: "40px minmax(0, 1fr)"',
    ),
    true,
  );
  assertEquals(
    managementSource.includes(
      'slot={scope === "service" ? "information" : "card"}',
    ),
    false,
  );
  for (const slot of ['slot="setup"', 'slot="empty"', 'slot="settings"']) {
    assertEquals(managementSource.includes(slot), true);
  }
  assertEquals(
    managementSource.includes(
      'type ProviderManagementLifecycleSlot = "setup" | "empty" | "settings"',
    ),
    true,
  );
  assertEquals(
    managementSource.includes("data-provider-management-actions"),
    true,
  );
  assertEquals(
    managementSource.includes("data-provider-management-footer"),
    true,
  );
  assertEquals(managementSource.includes("WebkitLineClamp: 2"), true);
});

Deno.test("Provider authentication copy dispatches on typed presentation, not Provider id", () => {
  assertEquals(managementSource.includes('case "account"'), true);
  assertEquals(managementSource.includes('case "api_key"'), true);
  assertEquals(managementSource.includes('empty: "API key missing"'), true);
  for (const provider of ["claude-deepseek", "codex-deepseek"]) {
    assertEquals(managementSource.includes(provider), false);
  }
});

Deno.test("Service authentication keeps Cowboy alive while Provider sign-in opens externally", () => {
  assertEquals(
    managementSource.includes("providerAuthenticationExecutorEntry("),
    true,
  );
  assertEquals(
    managementSource.includes("authenticationPendingMethod"),
    true,
  );
  assertEquals(managementSource.includes("authenticationError"), true);
  assertEquals(
    managementSource.includes('aria-label="Back to sign-in methods"'),
    true,
  );
  assertEquals(managementSource.includes('component="a"'), true);
  assertEquals(managementSource.includes('target="_blank"'), true);
  assertEquals(
    managementSource.includes("shouldRouteAuthenticationClick(event)"),
    true,
  );
  assertEquals(
    managementSource.includes("openAuthenticationUrl(challenge.verification_url)"),
    true,
  );
  assertEquals(managementSource.includes("closeAuthenticationBrowser()"), true);
});

Deno.test("Service credential management renders one card per typed authentication scope", () => {
  assertEquals(
    managementSource.includes("groupProviderAuthentications(latestEntries)"),
    true,
  );
  assertEquals(
    managementSource.includes("data-provider-credential-card"),
    true,
  );
  assertEquals(
    managementSource.includes(
      "providerCredentialTitle(credentialGroup.entries)",
    ),
    true,
  );
  assertEquals(
    managementSource.includes("data-provider-credential-consumers"),
    true,
  );
  assertEquals(
    managementSource.includes(
      "One Cowboy Service credential shared by ${credentialEntries.length} Providers",
    ),
    true,
  );
  assertEquals(managementSource.includes("entries.slice(0, 4)"), true);
});

Deno.test("Machine credential status gives a typed Service-level recovery action", () => {
  assertEquals(managementSource.includes('case "current"'), true);
  assertEquals(managementSource.includes('case "pending"'), true);
  assertEquals(
    managementSource.includes('"Credentials missing · Add API key above"'),
    true,
  );
  assertEquals(
    managementSource.includes('"Credentials missing · Sign in above"'),
    true,
  );
});

Deno.test("Provider marks preserve host component classes and compact chip spacing", () => {
  assertEquals(surfaceSource.includes("className?: string | undefined;"), true);
  assertEquals(surfaceSource.includes("className={className}"), true);
  assertEquals(
    managementSource.includes('"& .MuiChip-icon": { ml: 0.625, mr: 0.125 }'),
    true,
  );
});

Deno.test("Provider actions stay visually distinct from read-only chips", () => {
  assertEquals(
    managementSource.includes("data-provider-management-root"),
    true,
  );
  assertEquals(
    managementSource.includes('"& .MuiButton-root": {\n          borderRadius: 1,'),
    true,
  );
  assertEquals(
    managementSource.includes('"& .MuiButton-outlinedPrimary"'),
    true,
  );
  assertEquals(
    managementSource.includes('"& .MuiButton-outlinedError"'),
    true,
  );
  assertEquals(
    surfaceSource.includes(
      'effect?.capability === "logout_service_authentication"',
    ),
    true,
  );
  assertEquals(
    surfaceSource.includes("data-provider-destructive-action={destructive"),
    true,
  );
});

Deno.test("Provider vector marks preserve edge antialiasing inside compact chips", () => {
  assertEquals(
    surfaceSource.includes(
      'height: scaledSize,\n          minWidth: scaledSize,',
    ),
    true,
  );
});

Deno.test("Provider activity renderer consumes only typed generic strategies", () => {
  for (
    const strategy of [
      'case "progress_ring"',
      'case "glyph_cycle"',
      'case "terminal_prompt"',
      'case "asset_signal"',
      'case "asset_pulse"',
    ]
  ) {
    assertEquals(surfaceSource.includes(strategy), true);
  }
  for (
    const provider of [
      "claude-code",
      "codex-deepseek",
      "gemini",
      "grok",
    ]
  ) {
    assertEquals(surfaceSource.includes(provider), false);
  }
});

Deno.test("terminal activity keeps its Provider-defined prompt geometry", () => {
  assertEquals(surfaceSource.includes("width: 16,\n          height: 16,"), true);
  assertEquals(surfaceSource.includes("provider-terminal-prompt"), true);
  assertEquals(surfaceSource.includes("provider-terminal-caret"), true);
  assertEquals(surfaceSource.includes("terminalPromptMotion"), true);
});

Deno.test("Provider Transcript renderer consumes only closed presentation variants", () => {
  for (
    const variant of [
      'case "timeline"',
      'case "workcell"',
      'case "signal"',
      'case "terminal"',
    ]
  ) {
    assertEquals(transcriptPresentationSource.includes(variant), true);
  }
  assertEquals(
    transcriptPresentationSource.includes(
      "data-provider-thought-variant={presentation.variant}",
    ),
    true,
  );
  for (
    const provider of [
      '"claude-code"',
      '"codex"',
      '"codex-deepseek"',
      '"gemini"',
      '"grok"',
    ]
  ) {
    assertEquals(transcriptPresentationSource.includes(provider), false);
  }
});

Deno.test("terminal thought rows use a composed icon instead of a raw prompt", () => {
  assertEquals(transcriptPresentationSource.includes("TerminalRounded"), true);
  assertEquals(transcriptPresentationSource.includes("CircularProgress"), true);
  assertEquals(
    transcriptPresentationSource.includes('<Box component="span">›</Box>'),
    false,
  );
});

Deno.test("streaming thoughts show one active marker once a step exists", () => {
  assertEquals(
    transcriptPresentationSource.includes(
      "streaming && visible.length === 0 && presentation.active_label",
    ),
    true,
  );
});

Deno.test("Provider UI v1 loading uses a compact neutral compatibility fallback", () => {
  assertEquals(
    surfaceSource.includes(
      'slot === "loading" && manifest.ui.schema_version === 1',
    ),
    true,
  );
  assertEquals(surfaceSource.includes("<LegacyProviderActivity />"), true);
  assertEquals(surfaceSource.includes("const legacyResponsive"), true);
});

Deno.test("Provider activity keeps motion provider-authored and geometry renderer-owned", () => {
  assertEquals(
    surfaceSource.includes(
      "data-provider-activity-indicator={node.indicator.kind}",
    ),
    true,
  );
  assertEquals(
    surfaceSource.includes("data-provider-activity-effect={node.label.effect}"),
    true,
  );
  assertEquals(
    surfaceSource.includes(
      "readableProviderMarkColor(manifest.display.accent, theme)",
    ),
    true,
  );
  assertEquals(
    transcriptPresentationSource.includes(
      "size={SIGNAL_THOUGHT_MARK_SIZE}",
    ),
    true,
  );
  assertEquals(
    transcriptPresentationSource.includes(
      "size: SIGNAL_THOUGHT_MARK_SIZE,\n        gap: 6,\n        paddingLeft: 8,",
    ),
    true,
  );
  assertEquals(
    transcriptPresentationSource.includes(
      "gridTemplateColumns: signalHeader",
    ),
    true,
  );
  assertEquals(
    transcriptPresentationSource.includes(
      'pl: signalHeader ? `${geometry.paddingLeft}px` : 0',
    ),
    true,
  );
  assertEquals(
    transcriptPresentationSource.includes(
      "borderRadius: signalHeader && currentSurface ? 1.25 : 0",
    ),
    true,
  );
  const grok = JSON.parse(grokProviderSource) as {
    activity: { indicator: { kind: string; interval_ms: number } };
  };
  assertEquals(grok.activity.indicator, {
    kind: "mark_pulse",
    interval_ms: 1650,
  });
});
