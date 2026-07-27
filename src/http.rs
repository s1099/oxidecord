//! Minimal [`HttpClient`] for gpui.

use futures::{AsyncReadExt as _, FutureExt as _, future::BoxFuture};
use gpui::http_client::{AsyncBody, HttpClient, Url, http, http::HeaderValue};

use crate::runtime;

pub struct ReqwestClient {
    client: reqwest::Client,
    user_agent: HeaderValue,
}

impl ReqwestClient {
    pub fn new() -> Self {
        let client = reqwest::Client::builder()
            .user_agent("oxidecord/0.1")
            .build()
            .expect("failed to build reqwest client");
        Self {
            client,
            user_agent: HeaderValue::from_static("oxidecord/0.1"),
        }
    }
}

impl HttpClient for ReqwestClient {
    fn type_name(&self) -> &'static str {
        "ReqwestClient"
    }

    fn user_agent(&self) -> Option<&HeaderValue> {
        Some(&self.user_agent)
    }

    fn proxy(&self) -> Option<&Url> {
        None
    }

    fn send(
        &self,
        req: http::Request<AsyncBody>,
    ) -> BoxFuture<'static, anyhow::Result<http::Response<AsyncBody>>> {
        let client = self.client.clone();
        let handle = runtime::handle().clone();

        async move {
            let (parts, mut body) = req.into_parts();

            let mut body_bytes = Vec::new();
            body.read_to_end(&mut body_bytes).await?;

            let url = parts.uri.to_string();
            let method = reqwest::Method::from_bytes(parts.method.as_str().as_bytes())?;

            // Convert headers via bytes so this stays correct even if gpui and
            // reqwest happen to resolve to different `http` crate versions.
            let mut headers = reqwest::header::HeaderMap::new();
            for (name, value) in parts.headers.iter() {
                if let (Ok(n), Ok(v)) = (
                    reqwest::header::HeaderName::from_bytes(name.as_str().as_bytes()),
                    reqwest::header::HeaderValue::from_bytes(value.as_bytes()),
                ) {
                    headers.insert(n, v);
                }
            }

            // reqwest must run inside the Tokio runtime; gpui's executor is not
            // one. Hand the work off and await the result over a channel.
            let (tx, rx) = futures::channel::oneshot::channel();
            handle.spawn(async move {
                let result = async {
                    let response = client
                        .request(method, url)
                        .headers(headers)
                        .body(body_bytes)
                        .send()
                        .await?;

                    let status = response.status();
                    let headers = response.headers().clone();
                    let bytes = response.bytes().await?;
                    Ok::<_, reqwest::Error>((status, headers, bytes))
                }
                .await;
                let _ = tx.send(result);
            });

            let (status, headers, bytes) = rx.await??;

            let mut builder = http::Response::builder().status(status.as_u16());
            for (name, value) in headers.iter() {
                builder = builder.header(name.as_str(), value.as_bytes());
            }
            let response = builder.body(AsyncBody::from(bytes.to_vec()))?;
            Ok(response)
        }
        .boxed()
    }
}
