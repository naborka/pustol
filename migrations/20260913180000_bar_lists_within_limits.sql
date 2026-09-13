-- API refuses bar config domain rejects; over-limit list stops deploy here, not every request with server fault.
-- Numbers copy `pustol-domain` list and zone name limits at time of writing; domain owns them.

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
