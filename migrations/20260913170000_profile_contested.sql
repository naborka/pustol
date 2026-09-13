-- Telegram signs whole seconds; two profiles in one second have no order. Contested second rewrites and claims nothing until later-second payload.

alter table telegram_user
  add column profile_contested boolean not null default false,
  add constraint telegram_user_contested_profile_is_signed
    check (not profile_contested or profile_signed_at is not null);
