-- Make "a row's project must match its user's project" a schema rule.
--
-- # The hole this closes
--
-- `POST /v1/payments/transactions` takes a `to_user_id` from the caller.
-- Everything else about a request is derived from credentials — user_id
-- from the token, project_id from the API key — but the recipient cannot
-- be, because the whole point is paying somebody else.
--
-- Nothing checked that the recipient belonged to the caller's project.
-- `getOrCreateByUser(callerProject, someOtherProjectsUserId)` was accepted
-- by the database, because `wallets_user_id_fkey` only asserts that the
-- user exists SOMEWHERE. So one customer could mint a wallet inside their
-- own project keyed to another customer's user id and move money into it
-- — money the real owner can never see, since balances are read per
-- (project, user). Verified against a live database before writing this:
-- the INSERT was accepted.
--
-- # Why a constraint rather than a check in the service
--
-- payments-service could query auth.users before creating a wallet, and
-- that would work until the next caller of getOrCreateByUser forgot to.
-- The rule belongs where it cannot be forgotten, and it is the same
-- reasoning behind the composite (wallet, project_id) keys that already
-- stop cross-project transfers.
--
-- Composite foreign keys need a matching unique key on the parent, so
-- auth.users gains UNIQUE (project_id, id). It is redundant with the
-- primary key on id alone — that is what makes it safe — and it is what
-- lets a child row reference the PAIR.

ALTER TABLE auth.users ADD CONSTRAINT users_project_id_key UNIQUE (project_id, id);

-- payments.wallets is the table with the actual hole.
--
-- The old single-column FK is dropped rather than kept: it asserted a
-- strictly weaker fact ("this user exists") than the composite one
-- ("this user exists IN THIS PROJECT"), so keeping both would only cost
-- an extra index maintenance on every write.
ALTER TABLE payments.wallets DROP CONSTRAINT wallets_user_id_fkey;
ALTER TABLE payments.wallets
    ADD CONSTRAINT wallets_project_user_fkey
    FOREIGN KEY (project_id, user_id)
    REFERENCES auth.users(project_id, id)
    ON DELETE CASCADE;

-- The geo tables cannot be exploited the same way today, because their
-- user_id always comes from the token and the token is bound to a
-- project. They get the constraint anyway: "unexploitable given the
-- current call sites" is a property of today's code, and this makes it a
-- property of the schema instead. The cost is one index lookup on insert.
ALTER TABLE geo.locations DROP CONSTRAINT locations_user_id_fkey;
ALTER TABLE geo.locations
    ADD CONSTRAINT locations_project_user_fkey
    FOREIGN KEY (project_id, user_id)
    REFERENCES auth.users(project_id, id)
    ON DELETE CASCADE;

ALTER TABLE geo.geofences DROP CONSTRAINT geofences_user_id_fkey;
ALTER TABLE geo.geofences
    ADD CONSTRAINT geofences_project_user_fkey
    FOREIGN KEY (project_id, user_id)
    REFERENCES auth.users(project_id, id)
    ON DELETE CASCADE;

ALTER TABLE geo.safety_votes DROP CONSTRAINT safety_votes_user_id_fkey;
ALTER TABLE geo.safety_votes
    ADD CONSTRAINT safety_votes_project_user_fkey
    FOREIGN KEY (project_id, user_id)
    REFERENCES auth.users(project_id, id)
    ON DELETE CASCADE;
