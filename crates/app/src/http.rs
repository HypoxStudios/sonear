use std::sync::{Arc, OnceLock};

use futures_util::future::BoxFuture;
use gpui_kit::http_client::http::{HeaderValue, Response, StatusCode};
use gpui_kit::http_client::{self, AsyncBody, HttpClient, Request, Url};
use tokio::runtime::Handle;

use crate::cover_cache::{DiskCache, is_cover_host};

static COVER_CACHE: OnceLock<Arc<DiskCache>> = OnceLock::new();

/// The disk cache behind cover requests, once the app has set one up.
pub fn cover_cache() -> Option<&'static Arc<DiskCache>> {
    COVER_CACHE.get()
}

/// Serves GPUI's `img(url)` requests through reqwest on the tokio runtime,
/// because GPUI's default client fails every request. Cover art from
/// Spotify's CDNs goes through a bounded disk cache.
pub struct ReqwestClient {
    client: reqwest::Client,
    runtime: Handle,
    covers: Option<Arc<DiskCache>>,
}

impl ReqwestClient {
    pub fn new(runtime: Handle, covers: Option<Arc<DiskCache>>) -> Arc<Self> {
        if let Some(covers) = &covers {
            let _ = COVER_CACHE.set(covers.clone());
        }

        let client = reqwest::Client::builder()
            .connect_timeout(std::time::Duration::from_secs(10))
            .timeout(std::time::Duration::from_secs(60))
            .build()
            .unwrap_or_default();

        Arc::new(Self {
            client,
            runtime,
            covers,
        })
    }
}

fn response_from(bytes: Vec<u8>) -> http_client::Result<Response<AsyncBody>> {
    Ok(Response::builder()
        .status(StatusCode::OK)
        .body(AsyncBody::from_bytes(bytes.into()))?)
}

impl HttpClient for ReqwestClient {
    fn user_agent(&self) -> Option<&HeaderValue> {
        None
    }

    fn proxy(&self) -> Option<&Url> {
        None
    }

    fn send(
        &self,
        request: Request<AsyncBody>,
    ) -> BoxFuture<'static, http_client::Result<Response<AsyncBody>>> {
        let client = self.client.clone();
        let covers = self.covers.clone();
        let task = self.runtime.spawn(async move {
            let (parts, _) = request.into_parts();
            let method = reqwest::Method::from_bytes(parts.method.as_str().as_bytes())?;
            let cache_key = covers.as_ref().and_then(|_| {
                let cacheable = parts.method == gpui_kit::http_client::http::Method::GET
                    && parts.uri.host().is_some_and(is_cover_host);

                cacheable.then(|| parts.uri.to_string())
            });

            if let (Some(covers), Some(url)) = (&covers, &cache_key) {
                let covers = covers.clone();
                let url = url.clone();

                if let Ok(Some(bytes)) = tokio::task::spawn_blocking(move || covers.get(&url)).await
                {
                    return response_from(bytes);
                }
            }

            let response = client
                .request(method, parts.uri.to_string())
                .headers(parts.headers)
                .send()
                .await?;
            let status = StatusCode::from_u16(response.status().as_u16())?;
            let headers = response.headers().clone();
            let bytes = response.bytes().await?;

            if let (Some(covers), Some(url)) = (covers, cache_key)
                && status == StatusCode::OK
            {
                let stored = bytes.to_vec();

                tokio::task::spawn_blocking(move || covers.put(&url, &stored));
            }

            let mut built = Response::builder().status(status);

            if let Some(target) = built.headers_mut() {
                *target = headers;
            }

            Ok::<_, anyhow::Error>(built.body(AsyncBody::from_bytes(bytes))?)
        });

        Box::pin(async move { task.await? })
    }
}

#[cfg(test)]
mod tests {
    use futures_util::AsyncReadExt as _;
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
    use tokio::net::TcpListener;

    use super::*;

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn fetches_bodies_through_reqwest() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();

        tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut buffer = [0u8; 1024];

            let _ = stream.read(&mut buffer).await;
            stream
                .write_all(
                    b"HTTP/1.1 200 OK\r\ncontent-length: 5\r\nconnection: close\r\n\r\nhello",
                )
                .await
                .unwrap();
        });

        let client = ReqwestClient::new(Handle::current(), None);
        let mut response = client
            .get(
                &format!("http://127.0.0.1:{port}/cover.jpg"),
                AsyncBody::empty(),
                true,
            )
            .await
            .unwrap();
        let mut body = String::new();

        response.body_mut().read_to_string(&mut body).await.unwrap();

        assert!(response.status().is_success());
        assert_eq!(body, "hello");
    }
}
