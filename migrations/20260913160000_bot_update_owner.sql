-- Who took an update in hand.
--
-- A claim is written before the update is answered. When the claim was written but the process that
-- wrote it could not go on — the reply saying the claim was written was lost, or reading what the
-- answer needs failed — the update was fetched again, found claimed, taken for somebody else's, and
-- never answered. A claim now names the process that made it, and that process may take it again.
--
-- A claim written before this has an owner no process is, so only its age can release it, as before.

alter table bot_update add column owner uuid not null default gen_random_uuid();

alter table bot_update alter column owner drop default;
