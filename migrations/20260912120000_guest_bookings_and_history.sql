-- What guest holds depends on clock, not status (unmarked past booking holds nothing, no-show in grace does); index cannot see clock, so rule lives in store under bar lock.
-- Every settings save reads unfinished bookings by `ends_at` under bar lock; index avoids walking all history.

drop index booking_one_live_per_guest_per_shift;

create index booking_unfinished on booking (bar_id, ends_at)
  where status <> 'cancelled';
