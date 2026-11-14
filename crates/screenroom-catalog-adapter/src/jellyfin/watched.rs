use super::{LiveJellyfin, page_query, valid_id};
use crate::user_state::{
    self, Acquisition, MAX_WATCHED_OBSERVATIONS, UserStateAdapter, UserStateScope,
    WatchedObservation, WatchedSnapshot,
};
use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet};

const MAX_PAGE_REQUESTS: usize = 1_200;

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Page {
    items: Vec<Row>,
    total_record_count: u32,
    start_index: u32,
}
#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Row {
    id: String,
    #[serde(rename = "Type")]
    kind: String,
    user_data: Option<State>,
}
#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct State {
    played: Option<bool>,
}

impl LiveJellyfin {
    async fn watched_pass(
        &self,
        scope: &UserStateScope,
        budget: &mut usize,
    ) -> Result<BTreeMap<String, Option<bool>>, user_state::Error> {
        use user_state::Error;
        let mut result = BTreeMap::new();
        for library in &scope.library_ids {
            let mut offset = 0u32;
            let mut total = None;
            loop {
                *budget += 1;
                if *budget > MAX_PAGE_REQUESTS {
                    return Err(Error::Limit);
                }
                let mut query = page_query(library, offset, 100).map_err(|_| Error::Incomplete)?;
                query.push(("EnableUserData", "true".into()));
                query.push(("CollapseBoxSetItems", "false".into()));
                let page: Page = self
                    .json(&["Users", &scope.profile_id, "Items"], &query)
                    .await
                    .map_err(|_| Error::Unavailable)?;
                if page.total_record_count as usize > MAX_WATCHED_OBSERVATIONS {
                    return Err(Error::Limit);
                }
                if page.start_index != offset
                    || page.items.len() > 100
                    || total.is_some_and(|n| n != page.total_record_count)
                {
                    return Err(Error::Incomplete);
                }
                total = Some(page.total_record_count);
                let end = offset
                    .checked_add(page.items.len() as u32)
                    .ok_or(Error::Limit)?;
                if end > page.total_record_count
                    || (page.items.is_empty() && end < page.total_record_count)
                {
                    return Err(Error::Incomplete);
                }
                for row in page.items {
                    valid_id(&row.id).map_err(|_| Error::Incomplete)?;
                    let watched = match row.kind.as_str() {
                        "Movie" | "Episode" => row.user_data.and_then(|s| s.played),
                        // Container Played can be aggregate-derived by the provider.
                        "Season" | "Series" => None,
                        _ => return Err(Error::Incomplete),
                    };
                    if result.insert(row.id, watched).is_some() {
                        return Err(Error::Incomplete);
                    }
                    if result.len() > MAX_WATCHED_OBSERVATIONS {
                        return Err(Error::Limit);
                    }
                }
                if end == page.total_record_count {
                    break;
                }
                offset = end;
            }
        }
        Ok(result)
    }
}

impl UserStateAdapter for LiveJellyfin {
    fn supports_watched(&self) -> bool {
        true
    }
    fn acquire_watched<'a>(&'a self, scope: &'a UserStateScope) -> Acquisition<'a> {
        Box::pin(async move {
            use user_state::Error;
            if scope.library_ids.is_empty()
                || scope.library_ids.len() > 32
                || scope.library_ids.iter().collect::<BTreeSet<_>>().len()
                    != scope.library_ids.len()
            {
                return Err(Error::Incomplete);
            }
            self.validate_profile(&scope.profile_id)
                .await
                .map_err(|e| match e {
                    super::Error::InvalidScope => Error::ProfileUnavailable,
                    _ => Error::Unavailable,
                })?;
            let mut budget = 0;
            let first = self.watched_pass(scope, &mut budget).await?;
            let second = self.watched_pass(scope, &mut budget).await?;
            if first != second {
                return Err(Error::Incomplete);
            }
            self.validate_profile(&scope.profile_id)
                .await
                .map_err(|e| match e {
                    super::Error::InvalidScope => Error::ProfileUnavailable,
                    _ => Error::Unavailable,
                })?;
            WatchedSnapshot::complete(
                first
                    .into_iter()
                    .map(|(external_item_id, watched)| WatchedObservation {
                        external_item_id,
                        watched,
                    })
                    .collect(),
            )
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jellyfin::JellyfinConfig;
    use serde_json::{Value, json};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    fn profile(disabled: bool) -> Value {
        json!([{"Id":"profile","Name":"Viewer","Policy":{"IsDisabled":disabled,"IsHidden":false}}])
    }
    fn row(id: &str, played: Option<bool>) -> Value {
        json!({"Id":id,"Type":"Movie","UserData":{"Played":played}})
    }
    fn page(rows: Vec<Value>, start: u32, total: u32) -> Value {
        json!({"Items":rows,"StartIndex":start,"TotalRecordCount":total})
    }
    async fn scripted(
        responses: Vec<(u16, Value)>,
    ) -> (LiveJellyfin, tokio::task::JoinHandle<Vec<String>>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let task = tokio::spawn(async move {
            let mut requests = vec![];
            for (status, body) in responses {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut bytes = vec![0; 8192];
                let n = socket.read(&mut bytes).await.unwrap();
                requests.push(String::from_utf8(bytes[..n].to_vec()).unwrap());
                let body = body.to_string();
                socket.write_all(format!("HTTP/1.1 {status} Test\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
            }
            requests
        });
        let file = std::env::temp_dir().canonicalize().unwrap().join(format!(
            "watched-test-{}-{}",
            std::process::id(),
            address.port()
        ));
        tokio::fs::write(&file, "test-only-token").await.unwrap();
        super::super::tests::private_permissions(&file);
        let provider = LiveJellyfin::from_config(JellyfinConfig {
            source_key: "test".into(),
            base_url: format!("http://{address}/"),
            secret_file: file.clone(),
            timeout_ms: 1000,
            max_response_bytes: 1024 * 1024,
        })
        .await
        .unwrap();
        tokio::fs::remove_file(file).await.unwrap();
        (provider, task)
    }
    fn scope() -> UserStateScope {
        UserStateScope {
            profile_id: "profile".into(),
            library_ids: vec!["selected".into()],
        }
    }

    #[tokio::test]
    async fn exact_observation_capacity_completes_both_passes_and_overflow_is_rejected() {
        let mut responses = vec![(200, profile(false))];
        for _ in 0..2 {
            for offset in (0..50_000).step_by(100) {
                responses.push((
                    200,
                    page(
                        (offset..offset + 100)
                            .map(|id| row(&format!("item-{id}"), Some(true)))
                            .collect(),
                        offset,
                        50_000,
                    ),
                ));
            }
        }
        responses.push((200, profile(false)));
        let (provider, server) = scripted(responses).await;
        assert_eq!(
            provider
                .acquire_watched(&scope())
                .await
                .unwrap()
                .observations()
                .len(),
            50_000
        );
        assert_eq!(server.await.unwrap().len(), 1_002);

        let (provider, server) =
            scripted(vec![(200, profile(false)), (200, page(vec![], 0, 50_001))]).await;
        assert_eq!(
            provider.acquire_watched(&scope()).await.unwrap_err(),
            user_state::Error::Limit
        );
        assert_eq!(server.await.unwrap().len(), 2);
    }

    #[tokio::test]
    async fn exact_page_request_budget_is_shared_between_passes_and_stops_before_overflow_io() {
        let (provider, server) = scripted(vec![(200, page(vec![], 0, 0))]).await;
        let mut budget = 1_199;
        assert!(
            provider
                .watched_pass(&scope(), &mut budget)
                .await
                .unwrap()
                .is_empty()
        );
        assert_eq!(budget, 1_200);
        assert_eq!(
            provider
                .watched_pass(&scope(), &mut budget)
                .await
                .unwrap_err(),
            user_state::Error::Limit
        );
        assert_eq!(server.await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn explicit_states_paged_selected_libraries_and_no_resume_inference() {
        let mut rows: Vec<_> = (0..101)
            .map(|i| row(&format!("item-{i:03}"), Some(i % 2 == 0)))
            .collect();
        rows[2] = row("item-002", None);
        rows[3]["Type"] = json!("Series");
        rows[4]["Type"] = json!("Season");
        rows[5]["UserData"] = Value::Null;
        let p1 = page(rows[..100].to_vec(), 0, 101);
        let p2 = page(rows[100..].to_vec(), 100, 101);
        let (provider, task) = scripted(vec![
            (200, profile(false)),
            (200, p1.clone()),
            (200, p2.clone()),
            (200, p1),
            (200, p2),
            (200, profile(false)),
        ])
        .await;
        let snapshot = provider.acquire_watched(&scope()).await.unwrap();
        let states: BTreeMap<_, _> = snapshot
            .observations()
            .iter()
            .map(|o| (o.external_item_id.as_str(), o.watched))
            .collect();
        assert_eq!(states["item-000"], Some(true));
        assert_eq!(states["item-001"], Some(false));
        for id in ["item-002", "item-003", "item-004", "item-005"] {
            assert_eq!(states[id], None);
        }
        let requests = task.await.unwrap();
        for request in &requests[1..5] {
            assert!(request.starts_with("GET /Users/profile/Items?"));
            assert!(request.contains("ParentId=selected"));
            assert!(request.contains("EnableUserData=true"));
            assert!(request.contains("CollapseBoxSetItems=false"));
            assert!(request.contains("Limit=100"));
            assert!(!request.contains("Resume"));
            assert!(!request.contains("IsPlayed"));
        }
        assert!(requests[2].contains("StartIndex=100"));
    }

    #[tokio::test]
    async fn partial_duplicate_churn_error_and_disabled_never_publish_complete() {
        use user_state::Error;
        let first = page(vec![row("a", Some(true))], 0, 2);
        let single = page(vec![row("a", Some(true))], 0, 1);
        let cases = vec![
            (vec![(200, profile(true))], Error::ProfileUnavailable),
            (
                vec![(200, profile(false)), (503, json!({}))],
                Error::Unavailable,
            ),
            (
                vec![
                    (200, profile(false)),
                    (200, first.clone()),
                    (200, page(vec![], 1, 2)),
                ],
                Error::Incomplete,
            ),
            (
                vec![
                    (200, profile(false)),
                    (200, first.clone()),
                    (200, page(vec![row("a", Some(false))], 1, 2)),
                ],
                Error::Incomplete,
            ),
            (
                vec![
                    (200, profile(false)),
                    (200, first),
                    (200, page(vec![row("b", Some(false))], 1, 3)),
                ],
                Error::Incomplete,
            ),
            (
                vec![
                    (200, profile(false)),
                    (200, single.clone()),
                    (200, page(vec![row("a", Some(false))], 0, 1)),
                ],
                Error::Incomplete,
            ),
            (
                vec![
                    (200, profile(false)),
                    (200, single.clone()),
                    (200, single),
                    (200, profile(true)),
                ],
                Error::ProfileUnavailable,
            ),
        ];
        for (responses, expected) in cases {
            let (provider, task) = scripted(responses).await;
            assert_eq!(
                provider.acquire_watched(&scope()).await.unwrap_err(),
                expected
            );
            task.await.unwrap();
        }
    }
}
