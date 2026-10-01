-- Delete the two k8s tables nothing ever read.
--
-- `kube_namespace_stats` (728 MB / 22.0 M rows) and `kube_deployment_stats`
-- (4.9 GB / 151.1 M rows) were written on every scrape since the feature shipped and
-- read by exactly nothing: a repo-wide search finds only the two INSERTs in ingest.rs,
-- no SELECT in any handler, and neither table appears in `backup::DATA_TABLES`. The
-- Cluster page's namespace / workload / node breakdown is aggregated on read from
-- `kube_container_stats`, which already carries those columns.
--
-- They were not merely idle, they were harmful. Together they held 5.6 GB of a 17 GB
-- database whose cap is 20 GB, so they are what kept the data cap tripping; eviction
-- then reclaimed space from the largest tier, which is `kube_container_stats` — the one
-- table the UI does read. Measured effect: one 4.7 GB day-chunk of cluster history
-- deleted every day for three weeks straight, leaving every cluster chart cut off at
-- the same boundary while 5.6 GB of unread rows sat untouched.
--
-- Dropping them is not recoverable, and deliberately so: there is no reader to break,
-- no backup that contained them, and keeping them "just in case" is what caused the
-- data loss this migration exists to stop. If per-deployment replica health is wanted
-- later (desired/ready/available for rollout alerting), it should come back as a small
-- purpose-built table with a retention policy sized to an actual query — not as an
-- unbounded 365-day firehose.

DROP TABLE IF EXISTS kube_namespace_stats;
DROP TABLE IF EXISTS kube_deployment_stats;
