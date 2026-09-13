-- Attempts live on claim, not process memory; inbox drops update past retry limit. Takeover resets to 1.

alter table bot_update add column attempts integer not null default 1;

alter table bot_update alter column attempts drop default;
