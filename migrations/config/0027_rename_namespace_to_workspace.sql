-- Rename the RBAC "namespace" concept to "workspace" (see the rename commit).
--
-- This is a FORWARD migration, not an in-place edit of 0001. Migrations 0001-0017
-- keep their original "namespace" naming so their checksums still match databases
-- that applied them before the rename (an in-place edit made sqlx refuse to start
-- with "migration N was previously applied but has been modified"). Both paths
-- converge here:
--   * fresh DB   -> 0001..0026 create the *namespace* schema, then this renames it.
--   * existing DB-> 0001..0026 already applied (checksums match), only this runs.
-- After this migration the schema matches the Rust/SPA code, which speaks "workspace".

-- Core table + role enum.
ALTER TABLE namespaces RENAME TO workspaces;
ALTER TYPE ns_role RENAME TO ws_role;

-- Every table that referenced the workspace by id. Renaming a column also updates
-- the PRIMARY KEY / CHECK constraints and foreign keys that reference it; renaming
-- the target table leaves those FKs intact (they bind by object id, not by name).
ALTER TABLE memberships   RENAME COLUMN namespace_id TO workspace_id;
ALTER TABLE api_keys      RENAME COLUMN namespace_id TO workspace_id;
ALTER TABLE systems       RENAME COLUMN namespace_id TO workspace_id;
ALTER TABLE monitors      RENAME COLUMN namespace_id TO workspace_id;
ALTER TABLE channels      RENAME COLUMN namespace_id TO workspace_id;
ALTER TABLE status_pages  RENAME COLUMN namespace_id TO workspace_id;
ALTER TABLE exec_sessions RENAME COLUMN namespace_id TO workspace_id;
ALTER TABLE alerts        RENAME COLUMN scope_namespace_id TO scope_workspace_id;

-- Indexes, so a fresh DB and an upgraded DB end up with identical names.
ALTER INDEX idx_systems_namespace       RENAME TO idx_systems_workspace;
ALTER INDEX idx_monitors_namespace      RENAME TO idx_monitors_workspace;
ALTER INDEX idx_exec_sessions_namespace RENAME TO idx_exec_sessions_workspace;
