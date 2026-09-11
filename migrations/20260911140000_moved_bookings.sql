-- Staff move a booking to another time, and the guest is told.
--
-- Its own kind, not one of the bar's templates: this notice the system wrote itself, and a queue
-- that cannot tell them apart cannot answer why a guest was written to.
alter type notification_kind add value if not exists 'moved';
