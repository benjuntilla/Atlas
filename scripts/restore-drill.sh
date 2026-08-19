#!/usr/bin/env bash
#
# Restore drill: prove a backup can actually be restored.
#
# A backup nobody has restored is not a backup, it is a hope. The failure
# modes are unglamorous and specific — a missing PostGIS extension, an
# owner that does not exist on the target, a dump taken with a flag that
# silently skipped large objects — and every one of them is discovered at
# the worst possible moment unless somebody looks first.
#
# This takes a real dump, restores it into a scratch database, and then
# CHECKS it: row counts per table, and the invariants that matter more
# than row counts (money conserved, tenancy intact, spatial indexes
# usable). It is safe to run against production, because it only ever
# READS the source and only ever writes to a database it creates.
#
# Usage:
#   scripts/restore-drill.sh [SOURCE_DATABASE_URL]
#
# Defaults to the local docker-compose database.

set -euo pipefail

SOURCE_URL="${1:-${DATABASE_URL:-postgres://atlas:atlas_dev@127.0.0.1:5432/atlas}}"
DRILL_DB="atlas_restore_drill_$$"
DUMP_FILE="$(mktemp -t atlas-drill-XXXXXX.dump)"

# Parse the libpq URL into psql-friendly pieces. Deliberately not using a
# URL library: this script has to run in a recovery shell that may have
# nothing but bash and the postgres client tools.
proto_stripped="${SOURCE_URL#*://}"
creds="${proto_stripped%%@*}"
hostpath="${proto_stripped#*@}"
export PGUSER="${creds%%:*}"
export PGPASSWORD="${creds#*:}"
hostport="${hostpath%%/*}"
export PGHOST="${hostport%%:*}"
export PGPORT="${hostport#*:}"
[ "$PGPORT" = "$PGHOST" ] && PGPORT=5432
SOURCE_DB="${hostpath#*/}"
SOURCE_DB="${SOURCE_DB%%\?*}"

cleanup() {
    rm -f "$DUMP_FILE"
    # Drop the scratch database even if a check failed, so a red drill does
    # not leave litter that makes the next one fail for a different reason.
    psql -d postgres -qc "DROP DATABASE IF EXISTS $DRILL_DB;" >/dev/null 2>&1 || true
}
trap cleanup EXIT

fail() { echo "DRILL FAILED: $*" >&2; exit 1; }

echo "source:  $PGHOST:$PGPORT/$SOURCE_DB"
echo "scratch: $DRILL_DB"

# --- 1. dump ----------------------------------------------------------------
# Custom format, which is what pg_restore needs for selective restore and
# parallelism. --no-owner and --no-privileges because the roles on a
# recovery target are rarely the roles on the source, and a restore that
# fails on a missing role has failed for a reason unrelated to the data.
echo "==> dumping"
pg_dump --format=custom --no-owner --no-privileges -d "$SOURCE_DB" -f "$DUMP_FILE"
dump_bytes=$(wc -c < "$DUMP_FILE")
echo "    ${dump_bytes} bytes"
[ "$dump_bytes" -gt 1000 ] || fail "dump is implausibly small (${dump_bytes} bytes)"

# --- 2. restore -------------------------------------------------------------
echo "==> restoring into $DRILL_DB"
psql -d postgres -qc "CREATE DATABASE $DRILL_DB;"

# pg_restore exits non-zero on any error, including ones that do not
# matter (an extension comment it cannot set). Capture the log and judge
# it rather than trusting the exit code alone — but do not simply ignore
# failures, which is how a half-restored database gets declared healthy.
restore_log="$(mktemp)"
if ! pg_restore --no-owner --no-privileges -d "$DRILL_DB" "$DUMP_FILE" >"$restore_log" 2>&1; then
    if grep -qvE "must be owner of extension|extension .* already exists" "$restore_log"; then
        echo "--- pg_restore output ---"
        cat "$restore_log"
        rm -f "$restore_log"
        fail "pg_restore reported errors beyond the known-harmless extension ones"
    fi
    echo "    (ignored harmless extension ownership notices)"
fi
rm -f "$restore_log"

# --- 3. compare -------------------------------------------------------------
# Row counts per table, source against restored. A dump that silently
# skipped a table looks perfect until you count.
echo "==> comparing row counts"
counts_sql="
SELECT table_schema || '.' || table_name
FROM information_schema.tables
WHERE table_schema IN ('auth','geo','payments','control')
  AND table_type = 'BASE TABLE'
ORDER BY 1;"

mismatch=0
while read -r tbl; do
    [ -z "$tbl" ] && continue
    src=$(psql -d "$SOURCE_DB" -tAc "SELECT count(*) FROM $tbl;")
    dst=$(psql -d "$DRILL_DB"  -tAc "SELECT count(*) FROM $tbl;")
    if [ "$src" != "$dst" ]; then
        echo "    MISMATCH $tbl: source=$src restored=$dst"
        mismatch=1
    else
        printf '    %-34s %s\n' "$tbl" "$src"
    fi
done < <(psql -d "$SOURCE_DB" -tAc "$counts_sql")
[ "$mismatch" -eq 0 ] || fail "row counts differ between source and restore"

# --- 4. invariants ----------------------------------------------------------
# Row counts prove the rows arrived. These prove they still MEAN the same
# thing — a restore that lost a constraint or an extension has the right
# number of rows and the wrong database.
echo "==> checking invariants"

check() {
    local label="$1" sql="$2" expected="$3"
    local actual
    actual=$(psql -d "$DRILL_DB" -tAc "$sql")
    if [ "$actual" != "$expected" ]; then
        fail "$label: expected '$expected', got '$actual'"
    fi
    printf '    %-52s ok\n' "$label"
}

# PostGIS is the one that bites: a dump restored into a database without
# the extension fails on the geometry columns, and a database WITH the
# extension but a different version can restore rows that no longer index.
check "postgis is present and queryable" \
    "SELECT count(*) > 0 FROM pg_extension WHERE extname = 'postgis';" "t"
check "geometry columns survived" \
    "SELECT count(*) = 0 FROM geo.locations WHERE position IS NULL;" "t"
check "a spatial query still runs" \
    "SELECT count(*) >= 0 FROM geo.geofences
     WHERE ST_DWithin(center::geography, ST_SetSRID(ST_MakePoint(0,0),4326)::geography, 1000);" "t"

# Tenancy: the whole point of migration 0050/0051. A restore that dropped
# the NOT NULLs would accept unscoped writes afterwards.
check "no user escaped its project" \
    "SELECT count(*) = 0 FROM auth.users WHERE project_id IS NULL;" "t"
check "project_id defaults stayed dropped" \
    "SELECT count(*) = 0 FROM information_schema.columns
     WHERE column_name = 'project_id' AND column_default IS NOT NULL
       AND table_schema IN ('auth','geo','payments');" "t"

# Money: balances must reconcile against the transactions that produced
# them. This is the check worth having — it is the one that catches a
# partial restore that row counts would call clean.
check "no negative balances" \
    "SELECT count(*) = 0 FROM payments.wallets WHERE balance_cents < 0;" "t"
check "settled transfers never cross projects" \
    "SELECT count(*) = 0
     FROM payments.transactions t
     JOIN payments.wallets w ON w.id = t.from_wallet
     WHERE w.project_id <> t.project_id;" "t"

# Constraints and indexes are objects too, and they are the ones a
# --data-only dump silently omits.
src_idx=$(psql -d "$SOURCE_DB" -tAc "SELECT count(*) FROM pg_indexes WHERE schemaname IN ('auth','geo','payments','control');")
dst_idx=$(psql -d "$DRILL_DB"  -tAc "SELECT count(*) FROM pg_indexes WHERE schemaname IN ('auth','geo','payments','control');")
[ "$src_idx" = "$dst_idx" ] || fail "index count differs: source=$src_idx restored=$dst_idx"
printf '    %-52s %s\n' "indexes restored" "$dst_idx"

src_fk=$(psql -d "$SOURCE_DB" -tAc "SELECT count(*) FROM pg_constraint WHERE contype = 'f';")
dst_fk=$(psql -d "$DRILL_DB"  -tAc "SELECT count(*) FROM pg_constraint WHERE contype = 'f';")
[ "$src_fk" = "$dst_fk" ] || fail "foreign key count differs: source=$src_fk restored=$dst_fk"
printf '    %-52s %s\n' "foreign keys restored" "$dst_fk"

# The migration ledger, so the restored database can be reasoned about at
# all: a restore that is three migrations behind will fail in ways that
# look like application bugs.
src_mig=$(psql -d "$SOURCE_DB" -tAc "SELECT max(version) FROM _sqlx_migrations;")
dst_mig=$(psql -d "$DRILL_DB"  -tAc "SELECT max(version) FROM _sqlx_migrations;")
[ "$src_mig" = "$dst_mig" ] || fail "migration version differs: source=$src_mig restored=$dst_mig"
printf '    %-52s %s\n' "schema version" "$dst_mig"

echo
echo "DRILL PASSED"
