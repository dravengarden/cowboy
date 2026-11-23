-- Same column and index as PostgreSQL: the owner turn that admitted a call.
ALTER TABLE managed_agent_calls ADD COLUMN owner_turn INTEGER;
CREATE INDEX managed_call_owner_turn_idx ON managed_agent_calls (parent_session_id, owner_turn);
