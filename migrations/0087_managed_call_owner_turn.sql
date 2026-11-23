-- Which owner turn admitted a call: the event seq of the session's latest
-- owner prompt when the call was admitted, or -1 when the session had none.
-- NULL for calls admitted before this column existed. The per-turn call budget
-- counts calls sharing the current turn. Additive and ignored by predecessors.
ALTER TABLE managed_agent_calls ADD COLUMN owner_turn BIGINT;
CREATE INDEX managed_call_owner_turn_idx ON managed_agent_calls (parent_session_id, owner_turn);
