-- API refuses bar config domain rejects; over-limit text stops deploy here, not every request with server fault.
-- Numbers copy `pustol-domain` text limits at time of writing; domain owns them.

do $$
begin
  if exists (
    select 1 from bar
    where char_length(name) > 100
       or char_length(address) > 200
       or exists (select 1 from unnest(message_templates) as text where char_length(text) > 1000)
       or exists (select 1 from unnest(cancel_reasons) as text where char_length(text) > 200)
  ) then
    raise exception 'a bar has a name, address, message or cancellation reason longer than now allowed (100, 200, 1000, 200 characters); shorten it before upgrading';
  end if;
end $$;
