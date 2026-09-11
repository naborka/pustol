-- Leaving early, walking in, and a note for the shift.
--
-- Three facts the room already had and the schema could not say.
--
-- *A party that has left.* Until now a booking held its table for the whole promised window
-- whatever happened in it, so a table visibly empty at 21:20 stayed sold until 23:00. The same
-- was true of a party that never came: `no_show` was a note about history that released nothing.
-- Both are now recorded as a release moment, and the window a booking *occupies* is derived from
-- it — which is what makes "free now", the guest's slots, the walk-in offer and the timeline
-- agree, because all four read the same range.
--
-- *A party with no booking.* They exist, they hold a table, and a shift that does not record them
-- cannot report its own evening honestly.
--
-- *A note.* "День рождения", "У окна" — staff-facing, never sent to the guest.

-- Existing values keep their ordinals; the new ones are appended, so no stored row is reread.
alter type booking_status add value if not exists 'left';
alter type booking_source add value if not exists 'walk';

alter table booking add column note text;

-- The moment the table went back into the pool: when the party left, or when a no-show stopped
-- being worth holding for. Null means the booking holds its table for the whole promised window,
-- which is the ordinary case and the one that needs no explanation.
alter table booking add column left_at timestamptz;

comment on column booking.left_at is
  'When the table was released back to the room, within the promised window.';

alter table booking add constraint booking_left_at_within_window check (
  left_at is null or (left_at >= starts_at and left_at <= ends_at)
);

-- A release belongs to a booking that stopped needing its table. A `confirmed` or `arrived`
-- booking carrying one would be a table both held and freed, which is not a state of the room.
--
-- Written as the two statuses it excludes rather than the three it allows, because `left` was
-- created by this same migration and PostgreSQL will not let a new enum value be used before the
-- transaction that added it commits.
alter table booking add constraint booking_left_at_needs_a_settled_status check (
  left_at is null or (status <> 'confirmed' and status <> 'arrived')
);

-- Short enough to read at a glance on a row in a list, which is the only place it is ever shown.
alter table booking add constraint booking_note_is_short check (
  note is null or char_length(note) <= 120
);

-- THE invariant, moved onto the range the booking actually occupies.
--
-- `during` was the window promised to the guest, and until there was a way to give a table back
-- early the two were the same range. They are not any more: a party that leaves at 21:20 holds
-- 21:00–21:20 and the room may sell 21:20 onwards. Renaming rather than redefining, so that no
-- reader is left thinking the old column still means what it used to; an empty range (released at
-- the very minute it began) overlaps nothing, which is exactly right.
alter table booking drop constraint booking_one_party_per_table_at_a_time;
alter table booking drop column during;

alter table booking add column occupies tstzrange
  generated always as (tstzrange(starts_at, coalesce(left_at, ends_at), '[)')) stored;

comment on column booking.occupies is
  'The range this booking holds its table for: the promised window, cut short by left_at.';

alter table booking add constraint booking_one_party_per_table_at_a_time exclude using gist (
  table_id with =,
  occupies with &&
) where (status <> 'cancelled' and table_id is not null);
