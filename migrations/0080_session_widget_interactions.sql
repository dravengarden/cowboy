-- Widget interactions (docs/widgets.md §8): one row per decision a person sent
-- from a widget input. The decision itself travels as an ordinary user prompt
-- whose client message id is `cmid`; this row records which input, revision
-- and option labels it answered so the input turns inert on every device.
-- `retired` marks a decision whose input a later revision changed; it never
-- settles that input again, even if a later revision restores it.
-- Older Controllers ignore this table.
CREATE TABLE IF NOT EXISTS session_widget_interactions (
    session_id TEXT NOT NULL,
    widget_id TEXT NOT NULL,
    cmid TEXT NOT NULL,
    revision BIGINT NOT NULL,
    node_id TEXT NOT NULL,
    group_key TEXT NOT NULL,
    signature TEXT NOT NULL,
    labels TEXT NOT NULL,
    message TEXT NOT NULL,
    retired BOOLEAN NOT NULL DEFAULT FALSE,
    created_at_ms BIGINT NOT NULL,
    PRIMARY KEY (session_id, cmid),
    FOREIGN KEY (session_id, widget_id) REFERENCES session_widgets (session_id, widget_id) ON DELETE CASCADE
);
CREATE INDEX IF NOT EXISTS session_widget_interactions_widget
    ON session_widget_interactions (session_id, widget_id, created_at_ms);
