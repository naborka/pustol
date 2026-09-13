-- Phone or Telegram username a person answers; nobody reads bot chat. Null: show nothing. Form decided by `pustol-domain`.

alter table bar add column contact text;

alter table bar add constraint bar_contact_not_blank check (contact is null or btrim(contact) <> '');
