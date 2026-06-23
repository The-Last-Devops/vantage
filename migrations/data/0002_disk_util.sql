-- Disk I/O utilization (%util): fraction of the interval the busiest disk was
-- busy servicing I/O. Stored per sample alongside the CPU breakdown.
ALTER TABLE system_metrics ADD COLUMN disk_util DOUBLE PRECISION;
