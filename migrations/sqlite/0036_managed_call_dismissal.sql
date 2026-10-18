-- Keep admission receipts and budgets intact when a person clears Calls.
ALTER TABLE managed_agent_calls ADD COLUMN dismissed BOOLEAN NOT NULL DEFAULT FALSE;
