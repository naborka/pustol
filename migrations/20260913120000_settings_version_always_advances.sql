-- The bar's settings carry a version, and a save made from any other version is refused.
--
-- The version is `bar.updated_at`. A version has to change with every write, and `now()` is the
-- start of the transaction on a clock that can be stepped back, so a save could leave the value it
-- found. The bar's own trigger moves it forward on every write, whatever the clock says.

create or replace function advance_bar_version() returns trigger
language plpgsql as $$
begin
  new.updated_at := greatest(now(), old.updated_at + interval '1 microsecond');
  return new;
end;
$$;

drop trigger bar_set_updated_at on bar;

create trigger bar_advance_version before update on bar
  for each row execute function advance_bar_version();
