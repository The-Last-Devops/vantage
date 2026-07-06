-- Always have a 'default' workspace so a fresh install isn't empty.
-- Idempotent: does nothing if it already exists.
INSERT INTO workspaces (name) VALUES ('default') ON CONFLICT (name) DO NOTHING;
