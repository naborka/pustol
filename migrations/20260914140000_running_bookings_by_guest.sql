-- The bookings a guest still holds are read by account and end, among live bookings.

create index booking_running_by_guest on booking (telegram_user_id, ends_at)
  where status <> 'cancelled';
