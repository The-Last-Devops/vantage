-- New service-check kind: domain registration expiry (RDAP, falling back to WHOIS).
-- Not the TLS certificate — that is `tls`. 0001 is applied on live databases and must
-- never be edited, so the enum grows here.
ALTER TYPE public.monitor_kind ADD VALUE IF NOT EXISTS 'domain_expiry';
