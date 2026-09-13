-- Serves lookup of bookings guest still holds.

create index booking_running_by_guest on booking (telegram_user_id, ends_at)
  where status <> 'cancelled';
