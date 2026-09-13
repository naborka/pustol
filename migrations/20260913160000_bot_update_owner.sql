-- Claim names owner process, so it may retake own claim after lost insert reply or failed read; else update never answered.
-- Older claims get owner no process has; only age releases them.

alter table bot_update add column owner uuid not null default gen_random_uuid();

alter table bot_update alter column owner drop default;
