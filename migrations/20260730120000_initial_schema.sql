-- Pustol: table reservations for a bar, run from a Telegram Mini App.
--
-- The schema carries exactly two kinds of rule.
--
-- *Representability*: a booking cannot end before it starts, a table cannot seat a negative
-- number of people, a cancellation reason cannot be attached to a booking that is not cancelled.
-- These are cheap, static, and they do not repeat any business logic.
--
-- *Concurrency*: two parties cannot be given the same table at the same time. Application code
-- fundamentally cannot guarantee this — between reading availability and writing a booking,
-- another transaction can do the same — so it is enforced by an exclusion constraint, which makes
-- the overlap unrepresentable rather than merely checked.
--
-- Business policy (how late a bar may open, how long a turn may be, which party sizes are
-- offered) lives in one place: `pustol-domain`. Repeating it here would create a second source of
-- truth that drifts, and would put the reason a setting was refused somewhere the settings screen
-- cannot read it.

create extension if not exists btree_gist;

-- ---------------------------------------------------------------------------------------------
-- Shared helpers
-- ---------------------------------------------------------------------------------------------

create or replace function set_updated_at() returns trigger
language plpgsql as $$
begin
  new.updated_at := now();
  return new;
end $$;

comment on function set_updated_at() is
  'Keeps updated_at honest regardless of what the writer remembered to set.';

-- ---------------------------------------------------------------------------------------------
-- Telegram identities
-- ---------------------------------------------------------------------------------------------

-- One row per human Telegram account the system has ever seen, guest or staff.
--
-- Authorisation is by this id and never by username: a username can be released and claimed by
-- somebody else, which would hand the new owner whatever the old owner was allowed to do.
create table telegram_user (
  id bigint primary key,
  username text,
  first_name text not null,
  last_name text,
  language_code text,
  -- Learned from what the Bot API actually does. A guest who arrives through the Mini App has a
  -- chat with the bot by construction; one who later blocks it is discovered when a send fails.
  -- There is no call that asks Telegram whether a chat exists, so this is the only honest way to
  -- know, and it is a record of evidence rather than a permission.
  can_receive_messages boolean not null default true,
  -- The guest asked to be reminded. Separate from the above because consent and deliverability
  -- are different facts: a guest can want a reminder the bot is unable to deliver, and a bar that
  -- conflated the two would either nag people who declined or silently drop what they asked for.
  reminders_opted_in boolean not null default false,
  -- The guest said "not now". Remembered so the app asks once rather than every visit.
  reminder_prompt_dismissed_at timestamptz,
  first_seen_at timestamptz not null default now(),
  last_seen_at timestamptz not null default now(),

  constraint telegram_user_id_positive check (id > 0),
  constraint telegram_user_first_name_not_blank check (btrim(first_name) <> '')
);

comment on table telegram_user is 'Telegram accounts, keyed by Telegram''s own immutable user id.';

-- ---------------------------------------------------------------------------------------------
-- The bar and its rules
-- ---------------------------------------------------------------------------------------------

create table bar (
  id uuid primary key default gen_random_uuid(),
  name text not null,
  address text not null,
  -- An IANA zone name. Validated by the API against the tzdata the API itself computes with
  -- (chrono-tz), which is the only list whose agreement actually matters; a check against
  -- PostgreSQL's separately-versioned list could reject a zone the API handles perfectly well.
  timezone text not null,
  turn_minutes integer not null,
  slot_step_minutes integer not null,
  max_party integer not null,
  horizon_days integer not null,
  remind_hours integer not null,
  grace_minutes integer not null,
  zones text[] not null,
  message_templates text[] not null,
  cancel_reasons text[] not null,
  created_at timestamptz not null default now(),
  updated_at timestamptz not null default now(),

  constraint bar_name_not_blank check (btrim(name) <> ''),
  constraint bar_address_not_blank check (btrim(address) <> ''),
  constraint bar_timezone_not_blank check (btrim(timezone) <> ''),
  constraint bar_turn_positive check (turn_minutes > 0),
  constraint bar_slot_step_positive check (slot_step_minutes > 0),
  constraint bar_max_party_positive check (max_party > 0),
  constraint bar_horizon_positive check (horizon_days > 0),
  constraint bar_remind_positive check (remind_hours > 0),
  constraint bar_grace_positive check (grace_minutes > 0),
  constraint bar_has_zones check (cardinality(zones) > 0),
  constraint bar_has_templates check (cardinality(message_templates) > 0),
  constraint bar_has_reasons check (cardinality(cancel_reasons) > 0)
);

create trigger bar_set_updated_at before update on bar
  for each row execute function set_updated_at();

-- Opening hours belong to a weekday, never to the bar as a whole: there is no day-less open()
-- to use on the wrong day by accident.
--
-- `close_minutes` may exceed 1440. A bar shutting at 02:00 closes at minute 1560 of the service
-- day that opened the previous evening, and the booking taken at 01:00 belongs to that shift.
create table bar_hours (
  bar_id uuid not null references bar (id) on delete cascade,
  -- 0 = Sunday, matching chrono's num_days_from_sunday, so no translation layer can invert it.
  weekday smallint not null,
  open_minutes integer not null,
  close_minutes integer not null,
  closed boolean not null default false,

  primary key (bar_id, weekday),
  constraint bar_hours_weekday_in_week check (weekday between 0 and 6),
  constraint bar_hours_open_within_calendar_day check (open_minutes between 0 and 1439),
  constraint bar_hours_closes_after_opening check (close_minutes > open_minutes),
  constraint bar_hours_closes_within_service_day check (close_minutes <= 1680)
);

-- A week with a missing day would make every question about that day unanswerable. Deferred so
-- that a settings save may replace all seven rows inside one transaction.
create or replace function assert_bar_week_complete(target uuid) returns void
language plpgsql as $$
declare
  present integer;
begin
  -- A cascading delete of the bar itself removes the hours with it; there is nothing to complain
  -- about, and complaining would make bars undeletable.
  if not exists (select 1 from bar where id = target) then
    return;
  end if;
  select count(*) into present from bar_hours where bar_id = target;
  if present <> 7 then
    raise exception 'bar % has opening hours for % weekdays, not 7', target, present
      using errcode = 'integrity_constraint_violation';
  end if;
end $$;

create or replace function bar_hours_week_complete() returns trigger
language plpgsql as $$
begin
  perform assert_bar_week_complete(coalesce(new.bar_id, old.bar_id));
  return null;
end $$;

create or replace function bar_week_complete() returns trigger
language plpgsql as $$
begin
  perform assert_bar_week_complete(coalesce(new.id, old.id));
  return null;
end $$;

create constraint trigger bar_hours_week_is_complete
  after insert or update or delete on bar_hours
  deferrable initially deferred
  for each row execute function bar_hours_week_complete();

create constraint trigger bar_week_is_complete
  after insert on bar
  deferrable initially deferred
  for each row execute function bar_week_complete();

-- ---------------------------------------------------------------------------------------------
-- The room
-- ---------------------------------------------------------------------------------------------

-- Tables are retired, never deleted.
--
-- Two things depend on it. Bookings that already happened must keep resolving to the table they
-- happened at, or the shift history stops being evidence. And the printed number must never be
-- handed to a different table: `number` is derived from the maximum this bar has ever used,
-- retired rows included, so retiring table 15 and adding another gives 16.
create table bar_table (
  id uuid primary key default gen_random_uuid(),
  bar_id uuid not null references bar (id) on delete cascade,
  number integer not null,
  seats integer not null,
  zone text not null,
  retired_at timestamptz,
  created_at timestamptz not null default now(),

  unique (bar_id, number),
  -- Referenced by the composite foreign keys below, which is how a booking is stopped from
  -- pointing at a table in a different bar.
  unique (bar_id, id),
  constraint bar_table_number_positive check (number > 0),
  constraint bar_table_seats_positive check (seats > 0),
  constraint bar_table_zone_not_blank check (btrim(zone) <> '')
);

create index bar_table_live on bar_table (bar_id) where retired_at is null;

-- ---------------------------------------------------------------------------------------------
-- Bookings
-- ---------------------------------------------------------------------------------------------

create type booking_status as enum ('confirmed', 'arrived', 'no_show', 'cancelled');

-- Where the booking came from. A staff-entered booking has no Telegram user at all, which is
-- what makes "the bot cannot message this guest" a structural fact rather than a flag.
create type booking_source as enum ('app', 'staff');

create table booking (
  id uuid primary key default gen_random_uuid(),
  bar_id uuid not null references bar (id) on delete cascade,
  -- Null means orphan: a live booking the room can no longer seat, created when a table was
  -- closed or shrunk under it. It holds nothing and blocks nobody, and the admin screen shows it
  -- until somebody settles the debt. Leaving the old table id would render the booking on a
  -- closed table's row, which is a lie.
  table_id uuid,
  -- Which shift this belongs to. A business fact fixed when the booking is taken, not derivable
  -- from the instant: whether 01:00 belongs to Friday or Saturday depends on hours that may
  -- since have changed.
  service_date date not null,
  -- The window actually promised to the guest. Stored per booking rather than derived from the
  -- bar's current turn length, so that lengthening the turn applies to the next guest instead of
  -- retroactively extending everyone already seated — nobody is told after the fact that their
  -- table is needed for two hours longer than they agreed to.
  starts_at timestamptz not null,
  ends_at timestamptz not null,
  during tstzrange generated always as (tstzrange(starts_at, ends_at, '[)')) stored,
  party_size integer not null,
  guest_name text not null,
  guest_username text,
  telegram_user_id bigint references telegram_user (id) on delete restrict,
  status booking_status not null default 'confirmed',
  source booking_source not null,
  -- The reason the guest was given, snapshotted: the bar's list of reasons may be edited later,
  -- and the record must say what was actually sent.
  cancel_reason text,
  cancelled_at timestamptz,
  created_at timestamptz not null default now(),
  updated_at timestamptz not null default now(),

  foreign key (bar_id, table_id) references bar_table (bar_id, id) on delete restrict,
  constraint booking_window_not_empty check (ends_at > starts_at),
  constraint booking_party_positive check (party_size > 0),
  constraint booking_guest_name_not_blank check (btrim(guest_name) <> ''),
  constraint booking_cancellation_is_coherent check (
    (status = 'cancelled') = (cancelled_at is not null)
    and (cancel_reason is null or status = 'cancelled')
  ),
  constraint booking_from_the_app_has_a_telegram_user check (
    source <> 'app' or telegram_user_id is not null
  ),

  -- THE invariant. One table, one party, one moment — unrepresentable rather than merely
  -- checked, because no amount of application care can close the window between reading
  -- availability and writing the booking. Half-open ranges, so a party leaving at 20:00 and the
  -- next arriving at 20:00 do not collide. Cancelled bookings and orphans hold nothing.
  constraint booking_one_party_per_table_at_a_time exclude using gist (
    table_id with =,
    during with &&
  ) where (status <> 'cancelled' and table_id is not null)
);

create trigger booking_set_updated_at before update on booking
  for each row execute function set_updated_at();

-- A guest holds one booking per shift. The application enforces the stronger rule — one live
-- booking at a time, rebooking replaces it — which this cannot express because "upcoming"
-- depends on the clock; it stands as the structural half of that guarantee.
create unique index booking_one_live_per_guest_per_shift
  on booking (bar_id, telegram_user_id, service_date)
  where status <> 'cancelled' and telegram_user_id is not null;

create index booking_by_shift on booking (bar_id, service_date)
  where status <> 'cancelled';

-- No separate index on `during`: the exclusion constraint above already builds a GiST index over
-- it, and every read reaches bookings by service date. A second GiST index would be maintained on
-- every insert, cancellation and reconciliation move for nothing.

create index booking_by_guest on booking (telegram_user_id, starts_at)
  where telegram_user_id is not null and status <> 'cancelled';

-- ---------------------------------------------------------------------------------------------
-- Tables taken out of service
-- ---------------------------------------------------------------------------------------------

-- A whole shift at a time, which is how a bar actually thinks about it: the terrace is shut
-- tonight because it is raining tonight.
create table table_block (
  id uuid primary key default gen_random_uuid(),
  bar_id uuid not null,
  table_id uuid not null,
  service_date date not null,
  reason text not null,
  created_by bigint references telegram_user (id) on delete set null,
  created_at timestamptz not null default now(),

  unique (table_id, service_date),
  foreign key (bar_id, table_id) references bar_table (bar_id, id) on delete cascade,
  constraint table_block_reason_not_blank check (btrim(reason) <> '')
);

create index table_block_by_shift on table_block (bar_id, service_date);

-- ---------------------------------------------------------------------------------------------
-- Who may use the admin side
-- ---------------------------------------------------------------------------------------------

-- Invitations are written as usernames because that is what one member of staff can tell
-- another. Authorisation is by `telegram_user_id`, bound the first time the invited person opens
-- the app; from then on the username is only a label.
create table bar_staff (
  bar_id uuid not null references bar (id) on delete cascade,
  username text not null,
  username_lower text generated always as (lower(username)) stored,
  telegram_user_id bigint references telegram_user (id) on delete restrict,
  invited_at timestamptz not null default now(),
  bound_at timestamptz,

  primary key (bar_id, username_lower),
  constraint bar_staff_username_not_blank check (btrim(username) <> ''),
  constraint bar_staff_binding_is_coherent check ((telegram_user_id is null) = (bound_at is null))
);

-- One Telegram account cannot occupy two seats on the same roster.
create unique index bar_staff_one_seat_per_account
  on bar_staff (bar_id, telegram_user_id)
  where telegram_user_id is not null;

-- ---------------------------------------------------------------------------------------------
-- Outgoing messages
-- ---------------------------------------------------------------------------------------------

create type notification_kind as enum ('reminder', 'cancelled', 'staff_message');

-- An outbox. The bot lives outside the transaction that decides to send something, so the
-- decision is committed here and the delivery attempted afterwards; a crash between the two
-- loses a reminder rather than a booking.
create table notification (
  id uuid primary key default gen_random_uuid(),
  bar_id uuid not null references bar (id) on delete cascade,
  booking_id uuid not null references booking (id) on delete cascade,
  telegram_user_id bigint not null references telegram_user (id) on delete cascade,
  kind notification_kind not null,
  -- The exact text that will be sent, snapshotted at enqueue time: the bar's templates and
  -- reasons can be edited afterwards, and the guest must receive what staff actually chose.
  body text not null,
  scheduled_for timestamptz not null,
  sent_at timestamptz,
  gave_up_at timestamptz,
  attempts integer not null default 0,
  last_error text,
  created_at timestamptz not null default now(),

  constraint notification_body_not_blank check (btrim(body) <> ''),
  constraint notification_attempts_not_negative check (attempts >= 0),
  constraint notification_settles_once check (sent_at is null or gave_up_at is null)
);

-- One reminder per booking, however many times reconciliation touches it.
create unique index notification_one_reminder_per_booking
  on notification (booking_id)
  where kind = 'reminder';

create index notification_due on notification (scheduled_for)
  where sent_at is null and gave_up_at is null;

-- ---------------------------------------------------------------------------------------------
-- Access
-- ---------------------------------------------------------------------------------------------

-- Row level security on with no policies at all: the anon and authenticated roles a managed
-- PostgreSQL exposes to the internet can read and write nothing here, whatever the client does
-- with its key. Every write goes through the API, which is what makes the allocator the single
-- decider of who gets a table — a client that could insert a booking directly could seat itself
-- anywhere.
alter table telegram_user enable row level security;
alter table bar enable row level security;
alter table bar_hours enable row level security;
alter table bar_table enable row level security;
alter table booking enable row level security;
alter table table_block enable row level security;
alter table bar_staff enable row level security;
alter table notification enable row level security;

-- Belt as well as braces: on such a host these roles are granted table privileges by
-- default, and revoking them means a future policy added by accident still grants nothing.
do $$
declare
  role_name text;
begin
  foreach role_name in array array['anon', 'authenticated']
  loop
    if exists (select 1 from pg_roles where rolname = role_name) then
      execute format('revoke all on all tables in schema public from %I', role_name);
      execute format('revoke all on all sequences in schema public from %I', role_name);
      execute format('revoke all on all functions in schema public from %I', role_name);
    end if;
  end loop;
end $$;
