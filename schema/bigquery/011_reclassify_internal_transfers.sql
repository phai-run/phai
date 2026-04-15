-- Migration 011: reserved no-op
-- User-specific reclassification rules must not be embedded in shared migrations.
-- Store them in the private `rules` table instead.
SELECT 1;
