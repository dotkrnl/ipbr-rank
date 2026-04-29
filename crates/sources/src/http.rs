use std::time::Duration;

#[derive(Debug, Clone)]
pub struct ReqwestHttp {
    client: reqwest::Client,
}

impl Default for ReqwestHttp {
    fn default() -> Self {
        Self {
            client: reqwest::Client::new(),
        }
    }
}

const MAX_RETRIES: u32 = 6;
const INITIAL_BACKOFF_MS: u64 = 500;
const MAX_BACKOFF_MS: u64 = 60_000;

fn parse_retry_after(headers: &reqwest::header::HeaderMap) -> Option<Duration> {
    headers
        .get(reqwest::header::RETRY_AFTER)
        .and_then(|h| h.to_str().ok())
        .and_then(|s| s.parse::<u64>().ok())
        .map(Duration::from_secs)
}

async fn send_with_retry(
    builder: reqwest::RequestBuilder,
) -> Result<reqwest::Response, crate::SourceError> {
    let mut backoff = Duration::from_millis(INITIAL_BACKOFF_MS);
    let mut last_err: Option<String> = None;
    for attempt in 0..=MAX_RETRIES {
        let req = builder
            .try_clone()
            .expect("request builder cloneable for retries");
        match req.send().await {
            Ok(resp) => {
                let status = resp.status();
                if status == reqwest::StatusCode::TOO_MANY_REQUESTS && attempt < MAX_RETRIES {
                    let wait = parse_retry_after(resp.headers()).unwrap_or(backoff);
                    tokio::time::sleep(wait).await;
                    backoff = (backoff * 2).min(Duration::from_millis(MAX_BACKOFF_MS));
                    continue;
                }
                if status.is_server_error()
                    && status != reqwest::StatusCode::NOT_IMPLEMENTED
                    && attempt < MAX_RETRIES
                {
                    tokio::time::sleep(backoff).await;
                    backoff = (backoff * 2).min(Duration::from_millis(MAX_BACKOFF_MS));
                    continue;
                }
                return Ok(resp);
            }
            Err(err) => {
                last_err = Some(err.to_string());
                if attempt < MAX_RETRIES {
                    tokio::time::sleep(backoff).await;
                    backoff = (backoff * 2).min(Duration::from_millis(MAX_BACKOFF_MS));
                    continue;
                }
                return Err(crate::SourceError::Http(err.to_string()));
            }
        }
    }
    Err(crate::SourceError::Http(
        last_err.unwrap_or_else(|| "max retries exhausted".into()),
    ))
}

#[async_trait::async_trait]
impl crate::Http for ReqwestHttp {
    async fn get_json(
        &self,
        url: &str,
        headers: &[(&str, &str)],
    ) -> Result<serde_json::Value, crate::SourceError> {
        let mut req = self.client.get(url);
        for (name, value) in headers {
            req = req.header(*name, *value);
        }
        let resp = send_with_retry(req).await?;
        let resp = resp
            .error_for_status()
            .map_err(|err| crate::SourceError::Http(err.to_string()))?;
        resp.json()
            .await
            .map_err(|err| crate::SourceError::Http(err.to_string()))
    }

    async fn get_text(
        &self,
        url: &str,
        headers: &[(&str, &str)],
    ) -> Result<String, crate::SourceError> {
        let mut req = self.client.get(url);
        for (name, value) in headers {
            req = req.header(*name, *value);
        }
        let resp = send_with_retry(req).await?;
        let resp = resp
            .error_for_status()
            .map_err(|err| crate::SourceError::Http(err.to_string()))?;
        resp.text()
            .await
            .map_err(|err| crate::SourceError::Http(err.to_string()))
    }
}
