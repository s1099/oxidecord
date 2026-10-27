//! Minimal [`HttpClient`] for gpui.

use std::sync::OnceLock;

use futures::{AsyncReadExt as _, FutureExt as _, future::BoxFuture};
use gpui_kit::http_client::{AsyncBody, HttpClient, Url, http, http::HeaderValue};

use super::runtime;

const USER_AGENT: &str = "oxidecord/0.1";

/// Shared so every download reuses one connection pool.
pub fn client() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .user_agent(USER_AGENT)
            .build()
            .expect("failed to build reqwest client")
    })
}

pub struct ReqwestClient {
    user_agent: HeaderValue,
}

impl ReqwestClient {
    pub fn new() -> Self {
        Self {
            user_agent: HeaderValue::from_static(USER_AGENT),
        }
    }
}

impl HttpClient for ReqwestClient {
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
        let client = client();

        async move {
            let (parts, mut body) = req.into_parts();

            let mut body_bytes = Vec::new();
            body.read_to_end(&mut body_bytes).await?;

            let url = parts.uri.to_string();
            let method = reqwest::Method::from_bytes(parts.method.as_str().as_bytes())?;

            // Via bytes, since gpui and reqwest may resolve different `http` versions.
            let mut headers = reqwest::header::HeaderMap::new();
            for (name, value) in parts.headers.iter() {
                if let (Ok(n), Ok(v)) = (
                    reqwest::header::HeaderName::from_bytes(name.as_str().as_bytes()),
                    reqwest::header::HeaderValue::from_bytes(value.as_bytes()),
                ) {
                    headers.insert(n, v);
                }
            }

            // reqwest needs a Tokio context; gpui's executor isn't one.
            let (status, headers, bytes) = runtime::run(async move {
                let mut response = client
                    .request(method, url)
                    .headers(headers)
                    .body(body_bytes)
                    .send()
                    .await?;
                let status = response.status();
                let headers = std::mem::take(response.headers_mut());
                let bytes = response.bytes().await?;
                Ok::<_, reqwest::Error>((status, headers, bytes))
            })
            .await??;

            let mut builder = http::Response::builder().status(status.as_u16());
            for (name, value) in headers.iter() {
                builder = builder.header(name.as_str(), value.as_bytes());
            }
            let response = builder.body(AsyncBody::from_bytes(bytes))?;
            Ok(response)
        }
        .boxed()
    }
}
