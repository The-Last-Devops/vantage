-- Optional hard cap on the Data DB size, with auto-eviction of the oldest chunks.
-- Lives on the singleton app_settings row (id=1), alongside the other hub-wide
-- settings (s3, backup, audit_retention_days) — no separate table needed.
ALTER TABLE app_settings ADD COLUMN IF NOT EXISTS data_cap_limit_bytes bigint NOT NULL DEFAULT 10737418240;  -- 10 GB
ALTER TABLE app_settings ADD COLUMN IF NOT EXISTS data_cap_enabled boolean NOT NULL DEFAULT false;
-- Drop the earlier standalone table if a dev DB already ran the first version.
DROP TABLE IF EXISTS data_cap;
