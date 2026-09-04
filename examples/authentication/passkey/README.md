# Passkey plugin

WebAuthn credentials and external ceremonies live in this plugin's own
PostgreSQL schema (`plugin_passkey`) or SQLite file. Cowboy core still owns
the user row, session cookies, and reauth policy flags.

The account panel source of truth is `ui/PasskeysPanel.tsx`. Cowboy Web re-exports
it so settings/setup keep one implementation. Runtime `ui/index.js` mounts that
panel through the host kit; if the host has not registered it yet, the slot
falls back to the same component.

Core `user_passkeys` / `admin_passkeys` tables are not modified. On first
activation Cowboy copies existing rows into the plugin namespace, then treats
the plugin store as authoritative.
