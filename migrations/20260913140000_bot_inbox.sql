-- Durable claim per update, so restart or two pollers never answer twice. Whoever inserts row answers. Keyed by bot id: update ids count per bot.
-- `claimed_at` expires claims: Telegram keeps update one day, restarts ids from random number after quiet week.

create table bot_update (
  bot_id bigint not null,
  update_id bigint not null,
  claimed_at timestamptz not null,

  primary key (bot_id, update_id)
);

alter table bot_update enable row level security;

do $$
declare
  role_name text;
begin
  foreach role_name in array array['anon', 'authenticated']
  loop
    if exists (select 1 from pg_roles where rolname = role_name) then
      execute format('revoke all on bot_update from %I', role_name);
    end if;
  end loop;
end $$;
