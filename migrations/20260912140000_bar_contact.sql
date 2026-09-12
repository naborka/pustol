-- How guests reach a person at the bar.
--
-- "Написать бару" opened the bot's chat, which nobody reads: a party of eight wrote there and heard
-- nothing back. A phone number or a Telegram username a person answers is the only honest target.
-- Optional, because a bar that gives none should say nothing rather than point at the bot. Which of
-- the two it is, is decided by `pustol-domain`.

alter table bar add column contact text;

alter table bar add constraint bar_contact_not_blank check (contact is null or btrim(contact) <> '');
