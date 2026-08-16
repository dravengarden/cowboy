-- Crash incidents predate the critical severity policy. Reclassify only
-- unambiguous runtime, startup, and application crashes; ordinary command
-- failures and intentional interruptions retain their original severity.
UPDATE runtime_incidents
SET severity = 'critical',
    classification = CASE
      WHEN summary ILIKE '%did not become ready%'
        OR summary ILIKE '%exited before readiness%'
        OR summary ILIKE '%generation launch failed%'
      THEN 'worker_startup_failure'
      ELSE classification
    END,
    updated_at = now()
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
    OR summary ILIKE '%did not become ready%'
    OR summary ILIKE '%exited before readiness%'
    OR summary ILIKE '%generation launch failed%'
  );
