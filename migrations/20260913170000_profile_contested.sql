-- Whether the second the stored profile was signed in carried another profile as well.
--
-- Telegram stamps whole seconds. Two payloads of one account and one second can name it two ways,
-- and nothing says which came last. Neither rewrites the other, and until now the one stored could
-- still claim a seat under its username when sent again. Once a second is seen to carry two
-- profiles it rewrites nothing and claims nothing; a payload signed in a later second settles it.

alter table telegram_user
  add column profile_contested boolean not null default false,
  add constraint telegram_user_contested_profile_is_signed
    check (not profile_contested or profile_signed_at is not null);
