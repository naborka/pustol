-- The room version moves at most once per transaction.
--
-- No other transaction sees a state between two writes of one transaction, so one step is enough, and
-- a write touching many rows no longer updates the counter row once per row.

alter table room_version add column bumped_in xid8;

create or replace function advance_room_version() returns trigger
language plpgsql as $$
declare
  target uuid;
begin
  if tg_op = 'DELETE' then
    target := old.bar_id;
  else
    target := new.bar_id;
  end if;
  update room_version set version = version + 1, bumped_in = pg_current_xact_id()
   where bar_id = target and bumped_in is distinct from pg_current_xact_id();
  return null;
end $$;

create or replace function track_room_version_of_bar() returns trigger
language plpgsql as $$
begin
  if tg_op = 'INSERT' then
    insert into room_version (bar_id, version, bumped_in) values (new.id, 1, pg_current_xact_id());
  else
    update room_version set version = version + 1, bumped_in = pg_current_xact_id()
     where bar_id = new.id and bumped_in is distinct from pg_current_xact_id();
  end if;
  return null;
end $$;
