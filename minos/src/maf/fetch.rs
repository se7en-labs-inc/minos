use anyhow::{Context, Result};
use chrono::NaiveDate;
use serde::Deserialize;

// ---------------------------------------------------------------------------
// GitHub types
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct SearchResponse {
    total_count: usize,
    incomplete_results: bool,
    items: Vec<PrItem>,
}

#[derive(Debug, Deserialize)]
pub struct PrItem {
    pub number: u64,
    pub title: String,
    pub html_url: String,
}

#[derive(Debug, Deserialize)]
pub struct Release {
    pub tag_name: String,
    pub name: Option<String>,
    pub html_url: String,
    pub published_at: Option<String>,
}

impl Release {
    pub(super) fn display_name(&self) -> &str {
        self.name.as_deref().unwrap_or(&self.tag_name)
    }
    pub(super) fn date_str(&self) -> &str {
        self.published_at
            .as_deref()
            .and_then(|s| s.get(..10))
            .unwrap_or("")
    }
}

// ---------------------------------------------------------------------------
// GitHub fetching
// ---------------------------------------------------------------------------

pub(super) fn gh_client() -> Result<reqwest::Client> {
    minos::http::client_builder()
        .user_agent("minos-maf/0.1")
        .build()
        .context("build HTTP client")
}

pub async fn fetch_merged_prs(repo: &str, start: NaiveDate, end: NaiveDate) -> Result<Vec<PrItem>> {
    let client = gh_client()?;
    merged_prs(&client, "https://api.github.com", repo, start, end).await
}

fn validate_scope(repo: &str, start: NaiveDate, end: NaiveDate) -> Result<()> {
    anyhow::ensure!(start <= end, "report start date is after end date");
    let parts = repo.split('/').collect::<Vec<_>>();
    anyhow::ensure!(
        parts.len() == 2
            && parts.iter().all(|part| !part.is_empty()
                && part
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))),
        "repository must be OWNER/REPO"
    );
    Ok(())
}

async fn response_json<T: serde::de::DeserializeOwned>(response: reqwest::Response) -> Result<T> {
    if matches!(response.status().as_u16(), 403 | 429) {
        let retry = response
            .headers()
            .get("retry-after")
            .or_else(|| response.headers().get("x-ratelimit-reset"))
            .and_then(|value| value.to_str().ok())
            .unwrap_or("see upstream rate limits");
        anyhow::bail!("GitHub denied or rate-limited the request ({}; retry/reset: {retry}); no partial report generated", response.status());
    }
    minos::http::json(response).await.context("GitHub response")
}

async fn merged_prs(
    client: &reqwest::Client,
    api: &str,
    repo: &str,
    start: NaiveDate,
    end: NaiveDate,
) -> Result<Vec<PrItem>> {
    validate_scope(repo, start, end)?;
    let q = format!("repo:{repo} is:pr is:merged merged:{start}..{end}");
    let mut total = None;
    let mut output = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    for page in 1..=10 {
        let response = client
            .get(format!("{api}/search/issues"))
            .query(&[
                ("q", q.as_str()),
                ("per_page", "100"),
                ("sort", "created"),
                ("order", "asc"),
                ("page", page.to_string().as_str()),
            ])
            .header("Accept", "application/vnd.github+json")
            .send()
            .await
            .context("GitHub search request")?;
        let response: SearchResponse = response_json(response).await?;
        anyhow::ensure!(!response.incomplete_results, "GitHub search returned incomplete results; narrow the date range, no partial report generated");
        anyhow::ensure!(response.total_count <= 1000, "GitHub search exceeds its 1000-result limit; narrow the date range, no partial report generated");
        anyhow::ensure!(
            total.is_none_or(|total| total == response.total_count),
            "GitHub results changed while paginating; retry with a stable date range"
        );
        total = Some(response.total_count);
        let length = response.items.len();
        anyhow::ensure!(
            length <= 100,
            "GitHub returned more than requested page size"
        );
        for item in response.items {
            anyhow::ensure!(
                seen.insert(item.number),
                "duplicate GitHub PR while paginating; retry the report"
            );
            output.push(item);
        }
        anyhow::ensure!(
            output.len() <= response.total_count,
            "GitHub total_count disagrees with returned items"
        );
        if output.len() == response.total_count {
            return Ok(output);
        }
        anyhow::ensure!(
            length == 100,
            "GitHub search ended before total_count; no partial report generated"
        );
    }
    anyhow::bail!("GitHub search pagination did not complete")
}

pub async fn fetch_releases(repo: &str, start: NaiveDate, end: NaiveDate) -> Result<Vec<Release>> {
    let client = gh_client()?;
    releases(&client, "https://api.github.com", repo, start, end).await
}

async fn releases(
    client: &reqwest::Client,
    api: &str,
    repo: &str,
    start: NaiveDate,
    end: NaiveDate,
) -> Result<Vec<Release>> {
    validate_scope(repo, start, end)?;
    let mut output = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    for page in 1..=100 {
        let response = client
            .get(format!("{api}/repos/{repo}/releases"))
            .query(&[("per_page", "100"), ("page", page.to_string().as_str())])
            .header("Accept", "application/vnd.github+json")
            .send()
            .await
            .context("GitHub releases request")?;
        let releases: Vec<Release> = response_json(response).await?;
        let length = releases.len();
        anyhow::ensure!(
            length <= 100,
            "GitHub returned more than requested page size"
        );
        for release in releases {
            anyhow::ensure!(
                seen.insert(release.html_url.clone()),
                "duplicate GitHub release while paginating; retry the report"
            );
            if let Some(published) = &release.published_at {
                let date = chrono::DateTime::parse_from_rfc3339(published)
                    .context("invalid GitHub release publication date")?
                    .date_naive();
                if date >= start && date <= end {
                    output.push(release);
                }
            }
        }
        if length < 100 {
            return Ok(output);
        }
    }
    anyhow::bail!("GitHub releases exceed the 100-page report limit; no partial report generated")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    async fn server(responses: Vec<(u16, Value)>) -> (String, tokio::task::JoinHandle<()>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let api = format!("http://{}", listener.local_addr().unwrap());
        let task = tokio::spawn(async move {
            for (index, (status, body)) in responses.into_iter().enumerate() {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut request = Vec::new();
                loop {
                    let mut bytes = [0; 4096];
                    let length = socket.read(&mut bytes).await.unwrap();
                    assert!(length > 0);
                    request.extend_from_slice(&bytes[..length]);
                    if request.windows(4).any(|bytes| bytes == b"\r\n\r\n") {
                        break;
                    }
                }
                let request = String::from_utf8(request).unwrap();
                let path = request.split_whitespace().nth(1).unwrap();
                let url = reqwest::Url::parse(&format!("http://example.invalid{path}")).unwrap();
                assert_eq!(
                    url.query_pairs().find(|(key, _)| key == "page").unwrap().1,
                    (index + 1).to_string()
                );
                let body = body.to_string();
                socket.write_all(format!("HTTP/1.1 {status} Test\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
            }
        });
        (api, task)
    }

    fn prs(start: u64, end: u64) -> Vec<Value> {
        (start..end).map(|number| json!({"number": number, "title": format!("PR {number}"), "html_url": format!("https://example.org/pr/{number}")})).collect()
    }
    fn release(index: u64) -> Value {
        json!({"tag_name": format!("v{index}"), "name": null, "html_url": format!("https://example.org/releases/{index}"), "published_at": "2026-09-15T12:00:00Z"})
    }
    fn dates() -> (NaiveDate, NaiveDate) {
        ("2026-09-01".parse().unwrap(), "2026-09-30".parse().unwrap())
    }

    #[tokio::test]
    async fn retrieves_more_than_one_hundred_prs_and_releases() {
        let (start, end) = dates();
        let (api, task) = server(vec![
            (
                200,
                json!({"total_count": 101, "incomplete_results": false, "items": prs(0, 100)}),
            ),
            (
                200,
                json!({"total_count": 101, "incomplete_results": false, "items": prs(100, 101)}),
            ),
        ])
        .await;
        let result = merged_prs(&gh_client().unwrap(), &api, "example/project", start, end)
            .await
            .unwrap();
        assert_eq!(result.len(), 101);
        assert_eq!(result[100].number, 100);
        task.await.unwrap();

        let (api, task) = server(vec![
            (200, Value::Array((0..100).map(release).collect())),
            (200, json!([release(100)])),
        ])
        .await;
        assert_eq!(
            releases(&gh_client().unwrap(), &api, "example/project", start, end)
                .await
                .unwrap()
                .len(),
            101
        );
        task.await.unwrap();
    }

    #[tokio::test]
    async fn rejects_incomplete_search_limits_rate_limits_and_malformed_results() {
        let (start, end) = dates();
        for (status, body) in [
            (
                200,
                json!({"total_count": 1001, "incomplete_results": false, "items": []}),
            ),
            (
                200,
                json!({"total_count": 1, "incomplete_results": true, "items": prs(0, 1)}),
            ),
            (
                200,
                json!({"total_count": 2, "incomplete_results": false, "items": prs(0, 1)}),
            ),
            (
                200,
                json!({"total_count": 2, "incomplete_results": false, "items": [prs(0, 1)[0], prs(0, 1)[0]]}),
            ),
            (200, json!({"items": []})),
            (429, json!({"message": "rate limited"})),
        ] {
            let (api, task) = server(vec![(status, body)]).await;
            assert!(
                merged_prs(&gh_client().unwrap(), &api, "example/project", start, end)
                    .await
                    .is_err()
            );
            task.await.unwrap();
        }
        assert!(validate_scope("example/project injected:query", start, end).is_err());
        assert!(validate_scope("example/project", end, start).is_err());
    }
}

// ---------------------------------------------------------------------------
// Layout constants  (letter paper, 1" margins → 9360 twips usable)
// ---------------------------------------------------------------------------
