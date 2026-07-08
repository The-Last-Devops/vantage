-- A rule may target one object (monitor_id / system_id) OR every service/host in a
-- namespace. scope_kind: NULL = specific target; 'all_services' | 'all_hosts' = ns-wide.
-- A ns-wide rule fires when ANY matching target meets the condition and names them.
ALTER TABLE alerts ADD COLUMN scope_kind TEXT;
ALTER TABLE alerts ADD COLUMN scope_namespace_id UUID REFERENCES namespaces(id) ON DELETE CASCADE;

-- The old CHECK required a monitor or system; ns-wide rules have neither.
ALTER TABLE alerts DROP CONSTRAINT IF EXISTS alerts_check;
ALTER TABLE alerts ADD CONSTRAINT alerts_target_ck CHECK (
    monitor_id IS NOT NULL
    OR system_id IS NOT NULL
    OR (scope_kind IS NOT NULL AND scope_namespace_id IS NOT NULL)
);
