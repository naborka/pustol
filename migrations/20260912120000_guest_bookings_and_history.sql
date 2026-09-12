-- A guest who went home may come back the same night, and a settings save stops reading history.
--
-- *One booking per guest per shift* counted every status but `cancelled`. A party that has left, or
-- never came, holds nothing, yet it still blocked the same guest from booking later that evening —
-- and the refusal surfaced as a server fault, because nothing expected this constraint to fire.
-- Only a booking that still holds or awaits its table counts now.
--
-- *Unfinished bookings* are read by every settings save, under the bar's lock, by `ends_at`. No
-- index covered it, so each save walked every booking the bar had ever taken.

drop index booking_one_live_per_guest_per_shift;

create unique index booking_one_live_per_guest_per_shift
  on booking (bar_id, telegram_user_id, service_date)
  where status in ('confirmed', 'arrived') and telegram_user_id is not null;

create index booking_unfinished on booking (bar_id, ends_at)
  where status <> 'cancelled';
