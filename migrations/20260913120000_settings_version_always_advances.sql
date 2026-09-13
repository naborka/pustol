-- Settings version is `bar.updated_at`; `now()` is transaction start on clock that can step back, so trigger forces advance every write.

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
