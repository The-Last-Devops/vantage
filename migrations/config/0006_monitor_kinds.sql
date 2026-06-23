-- More service-check kinds. ADD VALUE is idempotent and (PG12+) txn-safe since we
-- only add here, not use the new values in this migration.
ALTER TYPE monitor_kind ADD VALUE IF NOT EXISTS 'postgres';
ALTER TYPE monitor_kind ADD VALUE IF NOT EXISTS 'redis';
ALTER TYPE monitor_kind ADD VALUE IF NOT EXISTS 'dns';
ALTER TYPE monitor_kind ADD VALUE IF NOT EXISTS 'rabbitmq';
