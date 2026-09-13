-- Answers arrive out of order; screen keeps highest room version. Triggers bump it so no write path skips; row lock stops duplicate numbers.
-- `bar` trigger is `after` and writes only here; `bar_advance_version` never touches this table, so no trigger loop.

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
