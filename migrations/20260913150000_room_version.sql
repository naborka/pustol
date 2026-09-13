-- How far each bar's room has moved on.
--
-- The shift screen draws an evening from whichever answer arrives, and answers arrive out of order: a
-- read sent before a write can land after the write's own answer and put back the room as it was.
-- Every change to what an evening shows — a booking, a table taken out of service, a table, the bar's
-- own row — now moves one counter forward, and a screen keeps the newest evening it has.
--
-- Moved by triggers rather than by the code that writes, so no write path, present or future, can
-- change the room without moving it. One row per bar, locked by the transaction that moves it until
-- that transaction ends, so two writers never hand out the same number.
--
-- The bar's own trigger is an `after` trigger that writes only here. `bar_advance_version` sets
-- `updated_at` before the row is written and knows nothing of this table, and nothing here writes
-- `bar`, so neither can fire the other.

create table room_version (
  bar_id uuid primary key references bar (id) on delete cascade,
  version bigint not null
);

insert into room_version (bar_id, version) select id, 1 from bar;

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
  update room_version set version = version + 1 where bar_id = target;
  return null;
end $$;

create or replace function track_room_version_of_bar() returns trigger
language plpgsql as $$
begin
  if tg_op = 'INSERT' then
    insert into room_version (bar_id, version) values (new.id, 1);
  else
    update room_version set version = version + 1 where bar_id = new.id;
  end if;
  return null;
end $$;

create trigger booking_advances_room_version
  after insert or update or delete on booking
  for each row execute function advance_room_version();

create trigger table_block_advances_room_version
  after insert or update or delete on table_block
  for each row execute function advance_room_version();

create trigger bar_table_advances_room_version
  after insert or update or delete on bar_table
  for each row execute function advance_room_version();

create trigger bar_advances_room_version
  after insert or update on bar
  for each row execute function track_room_version_of_bar();

alter table room_version enable row level security;

do $$
declare
  role_name text;
begin
  foreach role_name in array array['anon', 'authenticated']
  loop
    if exists (select 1 from pg_roles where rolname = role_name) then
      execute format('revoke all on room_version from %I', role_name);
    end if;
  end loop;
end $$;
