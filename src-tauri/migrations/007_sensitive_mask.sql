-- v1.0.1 P0#12: per-row mask flag for sensitive memories.
--
-- When `sponge::absorb` or `blackhole::compress_group` writes a
-- memory whose `content` matches the sensitive-content predicate
-- (api_key / password / secret / token / Bearer / long base64 /
-- JWT shape), the writer:
--
--   1. blanks `summary_2000` to the empty string, and
--   2. sets `masked = 1` on the row.
--
-- The `masked` column lets the front-end render a "🔒 secret"
-- badge instead of the (now empty) summary, and lets the
-- export / sync paths refuse to include the row in plaintext
-- exports.

ALTER TABLE memories ADD COLUMN masked INTEGER NOT NULL DEFAULT 0;

-- v1.0.1 P0#12: the per-row flag only takes effect after the
-- column is present.  No data backfill is performed here
-- (backfilling would itself leak secrets into logs / JSON
-- dumps).  Future writes consult the column.
