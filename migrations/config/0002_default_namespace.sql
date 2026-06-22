-- Always have a 'default' namespace so a fresh install isn't empty.
-- Idempotent: does nothing if it already exists.
INSERT INTO namespaces (name) VALUES ('default') ON CONFLICT (name) DO NOTHING;
