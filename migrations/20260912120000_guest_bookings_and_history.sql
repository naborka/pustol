-- A guest who went home may come back the same night, and a settings save stops reading history.
--
-- *One booking per guest per shift* was an index, and an index can only count statuses. What a guest
-- holds is a table still being held for them, and that depends on the clock: a confirmed booking
-- whose evening ended unmarked holds nothing, while a no-show still inside its grace period does.
-- Counting statuses refused the first and let the second book twice. The rule now lives in the store,
-- asked of the same occupancy every other reading of the room uses, inside the transactions that take
-- a booking or change attendance under the bar's lock.
--
-- *Unfinished bookings* are read by every settings save, under the bar's lock, by `ends_at`. No
-- index covered it, so each save walked every booking the bar had ever taken.

drop index booking_one_live_per_guest_per_shift;

create index booking_unfinished on booking (bar_id, ends_at)
  where status <> 'cancelled';
