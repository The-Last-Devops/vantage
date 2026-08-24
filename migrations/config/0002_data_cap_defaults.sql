-- New defaults for the Data-DB storage cap: ON, 20 GiB (was OFF, 10 GiB).
--
-- The old defaults meant a fresh install had no size ceiling at all: nothing evicted
-- until an admin found the switch, and the 10 GiB limit was unrelated to (and on the
-- chart's defaults smaller than) the data volume it was supposed to protect.
--
-- 0001 is applied on live databases and must never be edited, so adopt the new values
-- here — and ONLY where the row still holds the old default, so an admin's own choice
-- is never overwritten.
UPDATE public.settings SET value = 'true'::jsonb, updated_at = now()
 WHERE key = 'data_cap_enabled' AND value = 'false'::jsonb;

UPDATE public.settings SET value = '21474836480'::jsonb, updated_at = now()
 WHERE key = 'data_cap_limit_bytes' AND value = '10737418240'::jsonb;

-- A DB that predates the seed rows gets them now.
INSERT INTO public.settings (key, value)
VALUES ('data_cap_enabled', 'true'::jsonb), ('data_cap_limit_bytes', '21474836480'::jsonb)
ON CONFLICT (key) DO NOTHING;
