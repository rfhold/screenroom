use std::{collections::HashSet, future::Future, time::Duration};

#[derive(Clone, Copy)]
pub struct Limits {
    pub pages: usize,
    pub items: usize,
    pub timeout: Duration,
}
impl Default for Limits {
    fn default() -> Self {
        // Generous for the seed, bounded even for adversarial cached responses.
        Self {
            pages: 512,
            items: 10_000,
            timeout: Duration::from_secs(120),
        }
    }
}

pub async fn collect<T, F, Fut>(label: &str, limits: Limits, mut fetch: F) -> Result<Vec<T>, String>
where
    F: FnMut(String) -> Fut,
    Fut: Future<Output = Result<(Vec<T>, String), String>>,
{
    tokio::time::timeout(limits.timeout, async {
        let mut token = String::new();
        let mut tokens = HashSet::new();
        let mut items = Vec::new();
        for _ in 0..limits.pages {
            if !tokens.insert(token.clone()) {
                return Err(format!("{label}: repeated pagination token"));
            }
            let (page, next) = fetch(token).await?;
            if page.len() > limits.items.saturating_sub(items.len()) {
                return Err(format!("{label}: item budget exceeded ({})", limits.items));
            }
            items.extend(page);
            if next.is_empty() {
                return Ok(items);
            }
            token = next;
        }
        Err(format!("{label}: page budget exceeded ({})", limits.pages))
    })
    .await
    .map_err(|_| {
        format!(
            "{label}: overall time budget exceeded ({:?})",
            limits.timeout
        )
    })?
}
