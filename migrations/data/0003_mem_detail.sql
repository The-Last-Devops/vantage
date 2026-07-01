-- Memory breakdown (free -m style): available / buffers / cached / free, in bytes.
-- Nullable — rows from older agents simply leave them NULL. The rollup views pick
-- these up when data_admin recreates the ladder (it detects the new column).
ALTER TABLE system_metrics
    ADD COLUMN IF NOT EXISTS mem_available BIGINT,
    ADD COLUMN IF NOT EXISTS mem_buffers   BIGINT,
    ADD COLUMN IF NOT EXISTS mem_cached    BIGINT,
    ADD COLUMN IF NOT EXISTS mem_free      BIGINT;
