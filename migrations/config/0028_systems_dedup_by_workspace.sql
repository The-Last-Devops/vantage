-- Identify a system by (workspace_id, hostname), not (key_id, hostname).
--
-- A host is the same host regardless of which enrollment key it used. Keying on
-- key_id meant re-enrolling under a NEW key (the "Add system" flow mints one each
-- time) created a DUPLICATE row for the same hostname, leaving the old one behind
-- as a stale/offline ghost. Switch the identity to (workspace_id, hostname) so a
-- re-enroll updates the existing row (its key_id follows the latest report).

-- 1) Collapse existing duplicates: keep the most-recently-seen row per
--    (workspace_id, hostname), delete the older ghosts. Their time-series in the
--    data DB is orphaned by system_id and aged out by retention (never JOINed).
DELETE FROM systems
WHERE id NOT IN (
    SELECT DISTINCT ON (workspace_id, hostname) id
    FROM systems
    ORDER BY workspace_id, hostname, last_seen DESC NULLS LAST, id DESC
);

-- 2) Swap the uniqueness constraint. The old (key_id, hostname) index is what let
--    two keys hold the same hostname; the new one forbids that within a workspace.
DROP INDEX IF EXISTS systems_key_host;
CREATE UNIQUE INDEX systems_ws_host ON systems (workspace_id, hostname);
