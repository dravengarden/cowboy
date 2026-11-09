-- Owner-defined shared Machine browser profiles. Several browser identities
-- (named accounts or legacy default usage cards) of distinct account-usage
-- families may use one browser key, derived from the owner and share id.
-- A share with deleting_ms set is being revoked: it admits no member and no
-- open, and disappears once its last placement is revoked.
CREATE TABLE provider_browser_shares (
    owner_user_id TEXT NOT NULL,
    share_id TEXT NOT NULL,
    browser_key TEXT NOT NULL,
    label TEXT NOT NULL,
    created_ms BIGINT NOT NULL,
    revision BIGINT NOT NULL,
    deleting_ms BIGINT,
    PRIMARY KEY (owner_user_id, share_id),
    UNIQUE (browser_key)
);
-- Share membership: a member (`account:<id>` or `card:<card>`) belongs to at
-- most one share, and a share holds at most one member per family.
CREATE TABLE provider_browser_share_members (
    owner_user_id TEXT NOT NULL,
    member TEXT NOT NULL,
    share_id TEXT NOT NULL,
    family TEXT NOT NULL,
    PRIMARY KEY (owner_user_id, member),
    UNIQUE (owner_user_id, share_id, family)
);
