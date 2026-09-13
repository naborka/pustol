-- `pustol-domain` now bounds how many entries each of the bar's lists may hold, and how long a zone's
-- name may be.
--
-- The API refuses to run a bar whose stored configuration the domain would not accept, so a bar
-- already over a bound would answer every request with a server fault after this upgrade. Checked
-- once, here, so that case stops the deploy with a message instead. The numbers are the limits at the
-- time of writing; the domain, not this file, is where they live.

do $$
begin
  if exists (
    select 1 from bar
    where cardinality(message_templates) > 20
       or cardinality(cancel_reasons) > 20
       or cardinality(zones) > 20
       or exists (select 1 from unnest(zones) as zone where char_length(zone) > 40)
       or (select count(*) from bar_staff staff where staff.bar_id = bar.id) > 50
       or (select count(*) from bar_table t where t.bar_id = bar.id and t.retired_at is null) > 100
  ) then
    raise exception 'a bar has more than 20 guest messages, cancellation reasons or zones, more than 50 admins, more than 100 live tables, or a zone name longer than 40 characters; trim it before upgrading';
  end if;
end $$;
