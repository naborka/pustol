-- Which updates sent to the bot have already been taken in hand.
--
-- Progress lived only in the running process. A crash, or Telegram out of reach while stopping, left
-- the next process asking from the start and answering every guest a second time, and two processes
-- polling at once could both answer the same tap.
--
-- One row per update, claimed before it is answered: whoever inserts the row answers, and nobody
-- else does. Keyed by the bot's own id, because update ids are counted per bot. `claimed_at` says
-- whether a row still counts: Telegram keeps an update for a day, and after a quiet week counts ids
-- afresh from a random number, so an old claim could name an update nobody has seen.

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
