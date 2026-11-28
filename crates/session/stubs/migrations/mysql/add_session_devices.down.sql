alter table sessions drop index sessions_user_id, drop column user_id, drop column public_id, drop column user_agent, drop column ip, drop column last_seen_at;
