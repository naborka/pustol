-- How many times an update has been taken in hand by the process that holds its claim.
--
-- The inbox lets an update go once answering it has failed more times than it retries, counted here
-- so the count is the claim's and not one process's memory. A claim taken over starts again at one.

alter table bot_update add column attempts integer not null default 1;

alter table bot_update alter column attempts drop default;
