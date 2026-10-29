-- DeepSeek bills server-side Search and Fetch per request. Gateways report the
-- counts per usage event; NULL means the producer predates them.
ALTER TABLE provider_usage_events ADD COLUMN search_requests INTEGER
    CHECK (search_requests IS NULL OR search_requests BETWEEN 0 AND 100000);
ALTER TABLE provider_usage_events ADD COLUMN fetch_requests INTEGER
    CHECK (fetch_requests IS NULL OR fetch_requests BETWEEN 0 AND 100000);
