-- Normalize zero-defaulted cache_write_tokens to NULL on providers that do not
-- separately meter cache writes (preserving genuine 0 on claude-code and any
-- future positive cache_write_tokens on every provider).
UPDATE turn_cache_facts
SET cache_write_tokens = NULL,
    other_uncached_input_tokens = NULL
WHERE provider IN (
    'codex',
    'antigravity',
    'claude-deepseek',
    'codex-deepseek',
    'deepseek-harness',
    'grok'
)
  AND cache_write_tokens = 0;
