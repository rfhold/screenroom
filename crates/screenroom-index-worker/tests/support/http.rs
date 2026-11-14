use screenroom_backend::{
    Uuid,
    db::MIGRATOR,
    embedding::{EmbeddingClient, EmbeddingConfig},
    indexing::{Configuration, ConfigureRequest, IndexingRepository, StartRequest},
    ingest::{Policy, Run},
    sqlx,
    storage::{ObjectStorage, StorageConfig},
};
use screenroom_catalog_adapter::jellyfin::{ConfigurationRevision, JellyfinConfig, LiveJellyfin};
use screenroom_index_worker::{Step, Worker};
#[path = "../../../screenroom-backend/tests/support/catalog.rs"]
mod fixture_catalog;
use fixture_catalog::CatalogRepository;
use serde_json::json;
use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};

pub const PNG_A: &[u8] = &[
    137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 1, 0, 0, 0, 1, 8, 2, 0,
    0, 0, 144, 119, 83, 222, 0, 0, 0, 12, 73, 68, 65, 84, 120, 156, 99, 112, 72, 104, 0, 0, 2, 4,
    1, 33, 203, 215, 148, 162, 0, 0, 0, 0, 73, 69, 78, 68, 174, 66, 96, 130,
];
pub const PNG_B: &[u8] = &[
    137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 1, 0, 0, 0, 1, 8, 2, 0,
    0, 0, 144, 119, 83, 222, 0, 0, 0, 12, 73, 68, 65, 84, 120, 156, 99, 104, 72, 112, 0, 0, 2, 132,
    1, 33, 105, 203, 189, 226, 0, 0, 0, 0, 73, 69, 78, 68, 174, 66, 96, 130,
];
pub const PNG_C: &[u8] = &[
    137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 1, 0, 0, 0, 1, 8, 2, 0,
    0, 0, 144, 119, 83, 222, 0, 0, 0, 12, 73, 68, 65, 84, 120, 156, 99, 80, 112, 104, 0, 0, 1, 100,
    0, 225, 199, 19, 201, 246, 0, 0, 0, 0, 73, 69, 78, 68, 174, 66, 96, 130,
];

#[derive(Default)]
pub struct State {
    pub provider_status: AtomicUsize,
    pub embedding_status: AtomicUsize,
    pub storage_status: AtomicUsize,
    pub block_upload: AtomicUsize,
    pub upload_started: tokio::sync::Notify,
    pub upload_cancelled: tokio::sync::Notify,
    pub tag: AtomicUsize,
    pub single_changed: AtomicUsize,
    pub image: AtomicUsize,
    pub count: AtomicUsize,
    pub calls: AtomicUsize,
    pub embedding_calls: AtomicUsize,
    pub delta_reject: AtomicUsize,
    pub repeated_page: AtomicUsize,
    pub missing_parent: AtomicUsize,
    pub replacement_server: AtomicUsize,
    pub requests: Mutex<Vec<String>>,
}
pub struct Http {
    pub url: String,
    pub state: Arc<State>,
    task: tokio::task::JoinHandle<()>,
    token: std::path::PathBuf,
}
async fn request(socket: &mut tokio::net::TcpStream) -> (String, Vec<u8>) {
    let mut bytes = Vec::new();
    let end = loop {
        let mut chunk = [0; 8192];
        let n = socket.read(&mut chunk).await.unwrap();
        if n == 0 {
            return (String::new(), vec![]);
        }
        bytes.extend_from_slice(&chunk[..n]);
        if let Some(i) = bytes.windows(4).position(|v| v == b"\r\n\r\n") {
            break i + 4;
        }
        assert!(bytes.len() < 65536);
    };
    let headers = String::from_utf8(bytes[..end].to_vec()).unwrap();
    let length = headers
        .lines()
        .find_map(|s| {
            s.split_once(':')
                .filter(|(k, _)| k.eq_ignore_ascii_case("content-length"))
                .map(|(_, v)| v.trim().parse::<usize>().unwrap())
        })
        .unwrap_or(0);
    while bytes.len() < end + length {
        let mut chunk = [0; 8192];
        let n = socket.read(&mut chunk).await.unwrap();
        assert!(n > 0);
        bytes.extend_from_slice(&chunk[..n]);
    }
    (headers, bytes[end..end + length].to_vec())
}
impl Http {
    pub async fn new() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let state = Arc::new(State::default());
        state.count.store(6, Ordering::SeqCst);
        let shared = state.clone();
        let objects = Arc::new(Mutex::new(HashMap::<String, (Vec<u8>, String)>::new()));
        let task = tokio::spawn(async move {
            loop {
                let (mut socket, _) = listener.accept().await.unwrap();
                let state = shared.clone();
                let objects = objects.clone();
                tokio::spawn(async move {
                    let (headers, input) = request(&mut socket).await;
                    if headers.is_empty() {
                        return;
                    }
                    let mut parts = headers.lines().next().unwrap().split_whitespace();
                    let method = parts.next().unwrap();
                    let target = parts.next().unwrap();
                    state
                        .requests
                        .lock()
                        .unwrap()
                        .push(format!("{method} {target}"));
                    let mut status = 200;
                    let mut mime = "application/json".to_string();
                    let mut body = vec![];
                    let mut length = None;
                    if target.starts_with("/objects/") {
                        if state.storage_status.load(Ordering::SeqCst) > 0 {
                            status = state.storage_status.load(Ordering::SeqCst);
                        } else if method == "PUT" {
                            if state.block_upload.load(Ordering::SeqCst) > 0 {
                                state.upload_started.notify_one();
                                let mut byte = [0];
                                let _ = socket.read(&mut byte).await;
                                state.upload_cancelled.notify_one();
                                return;
                            }
                            let content_type = headers
                                .lines()
                                .find_map(|s| {
                                    s.split_once(':')
                                        .filter(|(k, _)| k.eq_ignore_ascii_case("content-type"))
                                        .map(|(_, v)| v.trim().to_string())
                                })
                                .unwrap();
                            objects
                                .lock()
                                .unwrap()
                                .insert(target.into(), (input, content_type));
                        } else if let Some((data, typ)) = objects.lock().unwrap().get(target) {
                            mime = typ.clone();
                            length = Some(data.len());
                            if method != "HEAD" {
                                body = data.clone();
                            }
                        } else {
                            status = 404;
                        }
                    } else if target == "/v1/embeddings" {
                        state.embedding_calls.fetch_add(1, Ordering::SeqCst);
                        status = state.embedding_status.load(Ordering::SeqCst).max(200);
                        let mut vector = vec![0.0f32; 1024];
                        vector[0] = 1.;
                        body = serde_json::to_vec(
                            &json!({"model":"test","data":[{"index":0,"embedding":vector}]}),
                        )
                        .unwrap();
                    } else {
                        assert_eq!(method, "GET");
                        assert!(headers.contains("MediaBrowser Token=\"test-only-secret\""));
                        assert!(!target.contains("test-only-secret"));
                        state.calls.fetch_add(1, Ordering::SeqCst);
                        if state.provider_status.load(Ordering::SeqCst) > 0 {
                            status = state.provider_status.load(Ordering::SeqCst);
                            body = b"private-provider-error".to_vec();
                        } else if target == "/System/Info" {
                            body=serde_json::to_vec(&json!({"Id":if state.replacement_server.load(Ordering::SeqCst)==0{"local-http-server"}else{"replacement"},"ServerName":"Local HTTP test","Version":"10.11.0"})).unwrap();
                        } else if target == "/System/Info/Public" {
                            body = serde_json::to_vec(&json!({"Version":"10.11.3"})).unwrap();
                        } else if target == "/Library/VirtualFolders" {
                            body=serde_json::to_vec(&json!([{"ItemId":"movies","Name":"Movies","CollectionType":"movies"},{"ItemId":"shows","Name":"Shows","CollectionType":"tvshows"}])).unwrap();
                        } else if target == "/Users" {
                            body=serde_json::to_vec(&json!([{"Id":"selected","Name":"Real selected profile","Policy":{"IsDisabled":false,"IsHidden":false}}])).unwrap();
                        } else if target.contains("/Images/") {
                            mime = "image/png".into();
                            body = match state.image.load(Ordering::SeqCst) {
                                0 => PNG_A,
                                1 => PNG_B,
                                _ => PNG_C,
                            }
                            .to_vec();
                        } else if target.starts_with("/Items?") {
                            let query = target.split_once('?').unwrap().1;
                            let args: HashMap<_, _> =
                                query.split('&').filter_map(|p| p.split_once('=')).collect();
                            let tag = match state.tag.load(Ordering::SeqCst) {
                                0 => "a",
                                1 => "b",
                                _ => "c",
                            };
                            if args.contains_key("MinDateLastSaved")
                                && state.delta_reject.load(Ordering::SeqCst) > 0
                            {
                                status = 400;
                                body = b"{}".to_vec();
                            } else if let Some(id) = args.get("Ids") {
                                let row = match *id {
                                    "season" => {
                                        json!({"Id":"season","Name":"Season","Type":"Season","ParentId":"series"})
                                    }
                                    "series" => {
                                        json!({"Id":"series","Name":"Series","Type":"Series","ParentId":"shows"})
                                    }
                                    id => {
                                        json!({"Id":id,"Name":format!("Item {id}"),"Type":"Movie","ParentId":"movies","ImageTags":{"Primary":tag}})
                                    }
                                };
                                let rows = if state.missing_parent.load(Ordering::SeqCst) > 0 {
                                    vec![]
                                } else {
                                    vec![row]
                                };
                                body=serde_json::to_vec(&json!({"TotalRecordCount":rows.len(),"Items":rows,"StartIndex":0})).unwrap();
                            } else {
                                let parent = args.get("ParentId").copied().unwrap_or("movies");
                                let offset = args
                                    .get("StartIndex")
                                    .and_then(|s| s.parse::<usize>().ok())
                                    .unwrap_or(0);
                                let limit = args
                                    .get("Limit")
                                    .and_then(|s| s.parse::<usize>().ok())
                                    .unwrap_or(10);
                                let total = state.count.load(Ordering::SeqCst);
                                let items:Vec<_>=(offset..(offset+limit).min(total)).map(|i|{
                                 let i=if state.repeated_page.load(Ordering::SeqCst)>0 {0}else{i};
                                 let tag=if state.single_changed.load(Ordering::SeqCst)>0 && i>0 {"a"}else{tag};
                                if parent=="shows" {json!({"Id":format!("episode{i}"),"Name":format!("Episode {i}"),"Type":"Episode","ParentId":"season"})}
                                else {json!({"Id":format!("movie{i}"),"Name":format!("Item movie{i}"),"Type":"Movie","ParentId":"movies","ImageTags":{"Primary":tag}})}
                            }).collect();
                                body=serde_json::to_vec(&json!({"Items":items,"StartIndex":offset,"TotalRecordCount":total})).unwrap();
                            }
                        } else {
                            status = 404;
                        }
                    }
                    let length = length.unwrap_or(body.len());
                    let extra = if status == 429 {
                        "Retry-After: 1\r\n"
                    } else {
                        ""
                    };
                    let _=socket.write_all(format!("HTTP/1.1 {status} Result\r\nContent-Length: {length}\r\nContent-Type: {mime}\r\n{extra}Connection: close\r\n\r\n").as_bytes()).await;
                    let _ = socket.write_all(&body).await;
                });
            }
        });
        let directory = std::env::temp_dir()
            .canonicalize()
            .unwrap()
            .join(format!("worker-test-{}", Uuid::new_v4()));
        std::fs::create_dir(&directory).unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700)).unwrap();
        let token = directory.join(format!("{}.key", Uuid::new_v4()));
        std::fs::write(&token, "test-only-secret").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&token, std::fs::Permissions::from_mode(0o600)).unwrap();
        }
        Self {
            url,
            state,
            task,
            token,
        }
    }
    pub async fn configuration_revision(&self) -> ConfigurationRevision {
        LiveJellyfin::from_config(JellyfinConfig {
            source_key: "owned".into(),
            base_url: self.url.clone(),
            secret_file: self.token.clone(),
            timeout_ms: 10_000,
            max_response_bytes: 8 * 1024 * 1024,
        })
        .await
        .unwrap()
        .configuration_revision()
        .unwrap()
    }
    pub fn credential_directory(&self) -> std::path::PathBuf {
        self.token.parent().unwrap().to_owned()
    }
    pub async fn worker(&self, pool: sqlx::PgPool, embedding: bool, revision: &str) -> Worker {
        let embedding = embedding.then(|| {
            EmbeddingClient::new(EmbeddingConfig {
                base_url: format!("{}/v1", self.url),
                model: "test".into(),
                api_key: None,
                dimensions: 1024,
                revision: revision.into(),
                timeout: Duration::from_secs(5),
            })
            .unwrap()
        });
        Worker::new(
            pool.clone(),
            embedding,
            ObjectStorage::new(StorageConfig {
                endpoint: self.url.clone(),
                region: "garage".into(),
                bucket: "objects".into(),
                access_key_id: "test".into(),
                secret_access_key: "test-only".into(),
                timeout: Duration::from_secs(5),
            })
            .unwrap(),
            format!("test-{}", Uuid::new_v4()),
        )
        .await
        .unwrap()
        .with_connections(screenroom_backend::connections::ConnectionRepository::new(
            pool,
            Some(
                screenroom_backend::connections::CredentialDirectory::new(
                    self.credential_directory(),
                )
                .unwrap(),
            ),
        ))
    }
}
impl Drop for Http {
    fn drop(&mut self) {
        self.task.abort();
        for entry in std::fs::read_dir(self.token.parent().unwrap()).unwrap() {
            let path = entry.unwrap().path();
            assert_eq!(path.extension().unwrap(), "key");
            assert!(Uuid::parse_str(path.file_stem().unwrap().to_str().unwrap()).is_ok());
            std::fs::remove_file(path).unwrap();
        }
        std::fs::remove_dir(self.token.parent().unwrap()).unwrap();
    }
}

pub struct Fixture {
    pub pool: sqlx::PgPool,
    admin: sqlx::PgPool,
    name: String,
    pub source: Uuid,
}
impl Fixture {
    pub fn database_url(&self) -> String {
        let bootstrap = std::env::var("SCREENROOM_W_TEST_DATABASE_URL").unwrap();
        format!("{}/{}", bootstrap.rsplit_once('/').unwrap().0, self.name)
    }
    pub async fn new(http: &Http) -> Self {
        let fixture = Self::empty().await;
        let pool = fixture.pool.clone();
        Self::seed(http, fixture, pool).await
    }
    pub async fn empty() -> Self {
        let admin =
            sqlx::PgPool::connect(&std::env::var("SCREENROOM_W_TEST_DATABASE_URL").unwrap())
                .await
                .unwrap();
        let name = format!("w_{}", Uuid::new_v4().simple());
        sqlx::query(&format!("CREATE DATABASE {name}"))
            .execute(&admin)
            .await
            .unwrap();
        let pool = sqlx::postgres::PgPoolOptions::new()
            .max_connections(5)
            .connect_with(admin.connect_options().as_ref().clone().database(&name))
            .await
            .unwrap();
        MIGRATOR.run(&pool).await.unwrap();
        Self {
            pool,
            admin,
            name,
            source: Uuid::nil(),
        }
    }
    async fn seed(http: &Http, fixture: Self, pool: sqlx::PgPool) -> Self {
        let source = CatalogRepository::new(pool.clone())
            .configure_source(
                "owned",
                "jellyfin",
                &json!({"adapter":"jellyfin"}),
                &json!({"read_only":true}),
            )
            .await
            .unwrap()
            .id;
        let repo = IndexingRepository::new(pool.clone());
        repo.enroll_locator(source, http.configuration_revision().await.as_str())
            .await
            .unwrap();
        let connection = Uuid::new_v4();
        let credential =
            Uuid::parse_str(http.token.file_stem().unwrap().to_str().unwrap()).unwrap();
        let revision: String = sqlx::query_scalar(
            "SELECT configuration->>'locator_revision' FROM catalog.sources WHERE id=$1",
        )
        .bind(source)
        .fetch_one(&pool)
        .await
        .unwrap();
        sqlx::query("INSERT INTO connections.jellyfin(id,source_id,server_id,server_name,server_version,base_url,credential_ref,revision,indexing_profile_id) VALUES($1,$2,'local-http-server','Local HTTP test','10.11.0',$3,$4,$5,'profile')")
            .bind(connection).bind(source).bind(&http.url).bind(credential).bind(&revision).execute(&pool).await.unwrap();
        sqlx::query("UPDATE catalog.sources SET configuration=configuration||$2 WHERE id=$1")
            .bind(source)
            .bind(json!({"connection_id":connection,"connection_generation":1}))
            .execute(&pool)
            .await
            .unwrap();
        for id in ["movies", "shows"] {
            let public = Self::library(id);
            CatalogRepository::new(pool.clone())
                .configure_library(source, &public, id, id)
                .await
                .unwrap();
        }
        Self {
            pool,
            admin: fixture.admin,
            name: fixture.name,
            source,
        }
    }
    pub fn library(id: &str) -> String {
        format!(
            "jellyfin:owned:{}",
            id.as_bytes()
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>()
        )
    }
    pub async fn policy(
        &self,
        mode: &str,
        artwork: bool,
        embedding: bool,
        strategy: &str,
    ) -> Policy {
        let repair = mode == "repair";
        let config:Configuration=serde_json::from_value(json!({"indexing":1,"enabled":true,"mode":mode,"cadence_seconds":60,"reconcile_seconds":3600,"overlap_seconds":120,
            "components":{"metadata":!repair,"artwork":artwork,"embeddings":embedding},"strategy":strategy,
            "scope":{"library_ids":[Self::library("movies")],"item_kinds":["movie","series"]},"operator_profile_id":"profile",
            "artwork":{"posters":{"skip_existing":!repair || strategy=="missing"}},
            "limits":{"work_requests":100,"work_records":3,"lifetime_requests":100000,"lifetime_records":100000,"deadline_seconds":3600,"work_timeout_seconds":30}})).unwrap();
        IndexingRepository::new(self.pool.clone())
            .configure(
                self.source,
                &ConfigureRequest {
                    request_id: Uuid::new_v4(),
                    existing: None,
                    configuration: config,
                },
            )
            .await
            .unwrap()
    }
    pub async fn start(&self, policy: &Policy) -> Run {
        IndexingRepository::new(self.pool.clone())
            .start(&StartRequest {
                request_id: Uuid::new_v4(),
                policy_id: policy.id,
                policy_version: policy.version,
                scope: None,
            })
            .await
            .unwrap()
    }
    pub async fn cleanup(self) {
        assert!(
            self.name
                .strip_prefix("w_")
                .is_some_and(|v| Uuid::parse_str(v).is_ok())
        );
        self.pool.close().await;
        sqlx::query(&format!("DROP DATABASE {} WITH (FORCE)", self.name))
            .execute(&self.admin)
            .await
            .unwrap();
        self.admin.close().await;
    }
}
pub async fn drain(worker: &Worker) {
    for _ in 0..100000 {
        match worker.step().await.unwrap() {
            Step::Idle => return,
            Step::Committed => {}
            step => panic!("unexpected {step:?}"),
        }
    }
    panic!("drain exceeded bound");
}
pub async fn ready(pool: &sqlx::PgPool, run: Uuid) {
    sqlx::query(
        "UPDATE queue.jobs SET available_at=clock_timestamp() WHERE scope_id=$1 AND state='ready'",
    )
    .bind(run)
    .execute(pool)
    .await
    .unwrap();
    sqlx::query("UPDATE ingest.source_cooldowns SET until_at=clock_timestamp() WHERE source_id=(SELECT source_id FROM ingest.runs WHERE id=$1)").bind(run).execute(pool).await.unwrap();
}
