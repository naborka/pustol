-- `pustol-domain` now bounds how long the bar's own texts may be.
--
-- The API refuses to run a bar whose stored configuration the domain would not accept, so a bar
-- already holding a longer text would answer every request with a server fault after this upgrade.
-- Checked once, here, so that case stops the deploy with a message instead. The numbers are the
-- limits at the time of writing; the domain, not this file, is where they live.

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
