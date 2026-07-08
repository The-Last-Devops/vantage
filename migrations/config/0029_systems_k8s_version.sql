-- Kubernetes server version (apiserver gitVersion, e.g. "v1.29.4") for k8s-cluster
-- systems, reported by the cluster agent. NULL for hosts and until first reported.
ALTER TABLE systems ADD COLUMN IF NOT EXISTS k8s_version TEXT;
