-- When Telegram signed the profile stored for an account.
--
-- A payload is accepted for an hour and carries the profile as it was when Telegram signed it. An
-- older payload used after a newer one wrote the old username back over the new. A profile is now
-- rewritten only by one signed no earlier than the one stored. Empty for an account whose profile no
-- payload has written yet.

alter table telegram_user add column profile_signed_at timestamptz;
