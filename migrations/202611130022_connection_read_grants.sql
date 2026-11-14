REVOKE SELECT ON connections.jellyfin FROM screenroom_user,screenroom_worker;
GRANT SELECT(id,source_id,server_id,server_name,base_url,credential_ref,revision,generation)
 ON connections.jellyfin TO screenroom_user,screenroom_worker;
