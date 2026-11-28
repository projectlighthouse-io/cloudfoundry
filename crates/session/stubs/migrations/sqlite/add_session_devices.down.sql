drop index sessions_user_id;
alter table sessions drop column user_id;
alter table sessions drop column public_id;
alter table sessions drop column user_agent;
alter table sessions drop column ip;
alter table sessions drop column last_seen_at;
