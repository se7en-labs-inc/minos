//! Shared bounds for external HTTP calls. Responses are limited while streaming,
//! including chunked bodies without a trustworthy Content-Length header.
use anyhow::{ensure, Context, Result};
use serde::de::DeserializeOwned;
use std::time::Duration;

pub const MAX_RESPONSE_BYTES: usize = 16 * 1024 * 1024;

pub fn client_builder() -> reqwest::ClientBuilder {
    reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(30))
}

/// Read a bounded body; the caller decides how to report the HTTP status.
pub async fn body_bytes(mut response: reqwest::Response) -> Result<Vec<u8>> {
    ensure!(
        response
            .content_length()
            .is_none_or(|size| size <= MAX_RESPONSE_BYTES as u64),
        "HTTP response exceeds size limit"
    );
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.context("read HTTP response")? {
        ensure!(
            chunk.len() <= MAX_RESPONSE_BYTES - bytes.len(),
            "HTTP response exceeds size limit"
        );
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

pub async fn bytes(response: reqwest::Response) -> Result<Vec<u8>> {
    body_bytes(response.error_for_status().context("HTTP error status")?).await
}

pub async fn json<T: DeserializeOwned>(response: reqwest::Response) -> Result<T> {
    serde_json::from_slice(&bytes(response).await?).context("invalid HTTP JSON response")
}

pub async fn text(response: reqwest::Response) -> Result<String> {
    String::from_utf8(bytes(response).await?).context("HTTP response is not valid UTF-8")
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[tokio::test]
    async fn errors_oversized_chunked_bodies_and_stalls_fail() {
        for mode in 0..4 {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = listener.local_addr().unwrap();
            let server = tokio::spawn(async move {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut request = [0; 4096];
                let count = socket.read(&mut request).await.unwrap();
                assert!(count > 0);
                match mode {
                    0 => socket
                        .write_all(b"HTTP/1.1 429 Limited\r\nContent-Length: 2\r\n\r\n{}")
                        .await
                        .unwrap(),
                    1 => socket
                        .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 999999999\r\n\r\n")
                        .await
                        .unwrap(),
                    2 => {
                        socket
                            .write_all(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n")
                            .await
                            .unwrap();
                        let block = vec![b' '; 64 * 1024];
                        for _ in 0..=MAX_RESPONSE_BYTES / block.len() {
                            if socket.write_all(b"10000\r\n").await.is_err()
                                || socket.write_all(&block).await.is_err()
                                || socket.write_all(b"\r\n").await.is_err()
                            {
                                break;
                            }
                        }
                    }
                    _ => std::future::pending::<()>().await,
                }
            });
            let response = client_builder()
                .build()
                .unwrap()
                .get(format!("http://{address}"))
                .timeout(if mode == 3 {
                    Duration::from_millis(200)
                } else {
                    Duration::from_secs(5)
                })
                .send()
                .await;
            match response {
                Ok(response) => assert!(json::<serde_json::Value>(response).await.is_err()),
                Err(error) => assert!(mode == 3 && error.is_timeout()),
            }
            server.abort();
        }
    }
}
