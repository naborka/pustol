-- The bar's settings version is a counter.
--
-- Every update of the bar's row moves `settings_version` forward by one, whatever the clock says. A
-- save is refused when the version it was made from is not the one stored. `updated_at` keeps saying
-- when the row was last written.

alter table bar add column settings_version bigint not null default 1;

create or replace function advance_settings_version() returns trigger
language plpgsql as $$
begin
  new.settings_version := old.settings_version + 1;
  new.updated_at := now();
  return new;
end;
$$;

drop trigger bar_advance_version on bar;

drop function advance_bar_version();

create trigger bar_advance_settings_version before update on bar
  for each row execute function advance_settings_version();
