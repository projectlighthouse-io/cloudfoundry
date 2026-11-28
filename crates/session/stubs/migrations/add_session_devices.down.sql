drop index sessions_user_id;
alter table sessions drop column user_id, drop column public_id, drop column user_agent, drop column ip, drop column last_seen_at;
