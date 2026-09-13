-- Payload valid one hour; older payload must not overwrite newer profile. Only payload signed no earlier rewrites. Null until first payload.

alter table telegram_user add column profile_signed_at timestamptz;
