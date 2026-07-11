-- The primary key already provides the exact (session_id, seq) btree used by
-- every history query. The duplicate index costs the same space and doubles
-- write amplification.
DROP INDEX IF EXISTS events_session_seq_idx;

-- Runtime telemetry advances sessions.next_seq but has no transcript meaning.
DELETE FROM events
WHERE payload->>'kind' = 'update'
  AND payload->'update'->>'sessionUpdate' IN ('usage_update', 'session_info_update');

-- Older cowboy versions stored every tool progress frame as a full row. Fold
-- the last frame into the stable initial tool_call row, then drop the deltas.
-- toolCallId is session-local and stable for a call.
CREATE TEMP TABLE cowboy_tool_compaction AS
SELECT DISTINCT ON (initial.session_id, initial.payload->'update'->>'toolCallId')
  initial.session_id,
  initial.seq AS initial_seq,
  initial.payload AS initial_payload,
  delta.payload AS final_payload
FROM events AS initial
JOIN events AS delta
  ON delta.session_id = initial.session_id
 AND delta.payload->>'kind' = 'update'
 AND delta.payload->'update'->>'sessionUpdate' = 'tool_call_update'
 AND delta.payload->'update'->>'toolCallId' = initial.payload->'update'->>'toolCallId'
WHERE initial.payload->>'kind' = 'update'
  AND initial.payload->'update'->>'sessionUpdate' = 'tool_call'
ORDER BY
  initial.session_id,
  initial.payload->'update'->>'toolCallId',
  delta.seq DESC;

UPDATE events AS event
SET payload = jsonb_set(
  compact.initial_payload,
  '{update}',
  (compact.initial_payload->'update')
    || (compact.final_payload->'update')
    || '{"sessionUpdate":"tool_call"}'::jsonb
)
FROM cowboy_tool_compaction AS compact
WHERE event.session_id = compact.session_id
  AND event.seq = compact.initial_seq;

DELETE FROM events AS event
USING cowboy_tool_compaction AS compact
WHERE event.session_id = compact.session_id
  AND event.payload->>'kind' = 'update'
  AND event.payload->'update'->>'sessionUpdate' = 'tool_call_update'
  AND event.payload->'update'->>'toolCallId' = compact.initial_payload->'update'->>'toolCallId';

DROP TABLE cowboy_tool_compaction;
