-- Optional hard cap on the Data DB size, with auto-eviction of the oldest chunks.
-- Single-row table (the CHECK + boolean PK enforce exactly one row).
CREATE TABLE data_cap (
    id boolean PRIMARY KEY DEFAULT true CHECK (id),
    limit_bytes bigint NOT NULL DEFAULT 10737418240,  -- 10 GB
    enabled boolean NOT NULL DEFAULT false
);
INSERT INTO data_cap (id) VALUES (true) ON CONFLICT DO NOTHING;
