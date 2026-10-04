import { persisted } from "@cowboy/state-store";

/** Per-device explicit Machine/project identity; empty uses the automatic root. */
export const newSessionProjectPreference = persisted(
  "cowboy:new-session-default-project",
  "",
  {
    serialize: (value) => value,
    deserialize: (raw) => raw,
  },
);
