-- Retire the legacy default usage cards (docs/legacy-card-retirement.md).
-- Every share holding a `card:<card>` member moves its revision, so a reset
-- captured before is refused, and the card members leave. Family bindings a
-- card left behind stay: they block that family until a confirmed profile
-- reset, so no named account inherits a card's cookies.
UPDATE provider_browser_shares SET revision = revision + 1
WHERE EXISTS (
    SELECT 1 FROM provider_browser_share_members m
    WHERE m.owner_user_id = provider_browser_shares.owner_user_id
      AND m.share_id = provider_browser_shares.share_id
      AND m.member LIKE 'card:%'
);
DELETE FROM provider_browser_share_members WHERE member LIKE 'card:%';
-- Card browser enablement has no reader any more. The table stays, empty,
-- for a rolled-back Controller.
DELETE FROM provider_legacy_browsers;
