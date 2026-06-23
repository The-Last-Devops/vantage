-- Per-namespace alert thresholds for the "Needs attention" triage view.
-- NULL = use the built-in defaults (warn 80 / crit 90). Stored as JSONB so the
-- shape can grow without a migration:
--   { "cpu_warn":80,"cpu_crit":90,"mem_warn":80,"mem_crit":90,"disk_warn":80,"disk_crit":90 }
ALTER TABLE namespaces ADD COLUMN thresholds JSONB;
