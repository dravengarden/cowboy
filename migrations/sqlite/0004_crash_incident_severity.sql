-- Keep SQLite diagnostics aligned with the PostgreSQL crash policy.
UPDATE runtime_incidents
SET severity = 'critical',
    classification = CASE
      WHEN lower(summary) LIKE '%did not become ready%'
        OR lower(summary) LIKE '%exited before readiness%'
        OR lower(summary) LIKE '%generation launch failed%'
      THEN 'worker_startup_failure'
      ELSE classification
    END,
    updated_at_ms = CAST(strftime('%s', 'now') AS INTEGER) * 1000
WHERE severity <> 'critical'
  AND (
    classification IN (
      'runtime_failure',
      'process_exit',
      'resource_exhaustion',
      'worker_startup_failure',
      'client_render_failure',
      'client_window_error',
      'client_unhandled_rejection'
    )
    OR (
      id LIKE 'lifecycle:%'
      AND classification IN ('protocol_failure', 'transport_failure')
    )
    OR lower(summary) LIKE '%did not become ready%'
    OR lower(summary) LIKE '%exited before readiness%'
    OR lower(summary) LIKE '%generation launch failed%'
  );
