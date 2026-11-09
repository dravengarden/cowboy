-- Owner enablement of the Machine browser of a legacy default usage card
-- (the Service-collected default account of an account-usage provider).
-- A row means enabled; no row is the default, off.
CREATE TABLE provider_legacy_browsers (
    owner_user_id TEXT NOT NULL,
    card_id TEXT NOT NULL,
    PRIMARY KEY (owner_user_id, card_id)
);
