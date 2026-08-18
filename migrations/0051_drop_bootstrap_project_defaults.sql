-- Multi-tenancy, phase 2 of 2 (contract): remove the bootstrap defaults.
--
-- 0050 gave every data-plane table a project_id that defaulted to a
-- bootstrap project, so the schema could ship ahead of the services and
-- existing INSERT statements kept working. Every writer now sets the
-- column explicitly:
--
--   auth.users                    ExposedUserRepository.create
--   geo.locations                 queries::locations::insert_location
--   geo.geofences                 queries::geofences::create
--   payments.wallets              ExposedWalletRepository.getOrCreateByUser
--   payments.transactions         ExposedTransactionRepository.insertPending
--   payments.transaction_events   ExposedAuditLog.record
--
-- geo.safety_votes and geo.safety_ratings have no writer at all yet —
-- safety scoring is not built — so they are contracted here too rather
-- than left with a default nobody will remember when it is.
--
-- # Why the defaults have to go
--
-- A default tenant is a footgun, and specifically the quiet kind. An
-- INSERT that forgets project_id does not fail; it succeeds, and the row
-- lands in the bootstrap project where the tenant that should own it can
-- never see it and the operator has no signal anything went wrong. The
-- column only starts enforcing something once omitting it is an error.
--
-- The NOT NULL constraints stay. This drops only the defaults, so the
-- failure mode for a forgotten project_id changes from "silently wrong
-- tenant" to "null value in column violates not-null constraint", which
-- is a stack trace pointing at the exact INSERT.
--
-- Ordering note: this must not be applied before the services that set
-- the column are deployed. Applying it against an older build breaks
-- every write. That is the normal expand/contract bargain — the expand
-- half is safe in either order, the contract half is not.

ALTER TABLE auth.users                  ALTER COLUMN project_id DROP DEFAULT;
ALTER TABLE geo.locations               ALTER COLUMN project_id DROP DEFAULT;
ALTER TABLE geo.geofences               ALTER COLUMN project_id DROP DEFAULT;
ALTER TABLE geo.safety_votes            ALTER COLUMN project_id DROP DEFAULT;
ALTER TABLE geo.safety_ratings          ALTER COLUMN project_id DROP DEFAULT;
ALTER TABLE payments.wallets            ALTER COLUMN project_id DROP DEFAULT;
ALTER TABLE payments.transactions       ALTER COLUMN project_id DROP DEFAULT;
ALTER TABLE payments.transaction_events ALTER COLUMN project_id DROP DEFAULT;

-- The bootstrap project itself is deliberately NOT deleted. Rows created
-- before 0051 point at it, and dropping it would cascade them away —
-- which is a data-loss migration wearing a cleanup's clothes. Removing it
-- is an operator decision made against a real database, not something a
-- migration should decide on everyone's behalf.
