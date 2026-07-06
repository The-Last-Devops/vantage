-- System-level read-only role ("admin read"): can view every workspace but not
-- write. Combined with is_admin (full) and per-workspace memberships, this gives:
--   is_admin       → write everywhere
--   read_all       → read everywhere (no write)
--   neither        → per-workspace viewer/editor via memberships
ALTER TABLE users ADD COLUMN read_all BOOLEAN NOT NULL DEFAULT false;
