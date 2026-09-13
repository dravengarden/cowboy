use super::*;
use reqwest::{Client, Method, StatusCode, header};
use serde_json::{Value, json};

pub(super) struct Http {
    base: String,
    client: Client,
    cookie: Option<String>,
}

pub(super) struct Reply {
    pub status: StatusCode,
    pub value: Value,
    cookie: Option<String>,
}

impl Reply {
    pub fn ok(self) -> Result<Value, Failure> {
        if self.status == StatusCode::OK && !self.value.is_null() {
            Ok(self.value)
        } else {
            Err(Failure::WrongObservation)
        }
    }
}

impl Http {
    pub fn new(address: std::net::SocketAddr) -> Result<Self, Failure> {
        Ok(Self {
            base: format!("http://{address}"),
            cookie: None,
            client: Client::builder()
                .no_proxy()
                .redirect(reqwest::redirect::Policy::none())
                .connect_timeout(Duration::from_secs(1))
                .timeout(Duration::from_secs(60))
                .build()
                .map_err(|_| Failure::Setup)?,
        })
    }

    pub async fn call(
        &self,
        method: Method,
        path: &str,
        body: Option<Value>,
    ) -> Result<Reply, Failure> {
        let mut request = self
            .client
            .request(method, format!("{}{path}", self.base))
            .header(header::ORIGIN, &self.base);
        if let Some(cookie) = &self.cookie {
            request = request.header(header::COOKIE, cookie);
        }
        if let Some(body) = body {
            request = request.json(&body);
        }
        let mut response = request
            .send()
            .await
            .map_err(|_| Failure::WrongObservation)?;
        let status = response.status();
        if path.starts_with("/api/telemetry/")
            && status == StatusCode::OK
            && response
                .headers()
                .get(header::CACHE_CONTROL)
                .and_then(|h| h.to_str().ok())
                != Some("no-store")
        {
            return Err(Failure::WrongObservation);
        }
        let cookies: Vec<_> = response
            .headers()
            .get_all(header::SET_COOKIE)
            .iter()
            .filter_map(|h| h.to_str().ok()?.split(';').next())
            .filter(|h| h.starts_with("cowboy_user="))
            .map(str::to_owned)
            .collect();
        if cookies.len() > 1 {
            return Err(Failure::WrongObservation);
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| Failure::WrongObservation)?
        {
            if bytes.len() + chunk.len() > 64 * 1024 {
                return Err(Failure::WrongObservation);
            }
            bytes.extend_from_slice(&chunk);
        }
        // Auth middleware may return a bounded plain-text/empty denial. Only
        // status is used there; successful telemetry views must be valid JSON.
        Ok(Reply {
            status,
            value: serde_json::from_slice(&bytes).unwrap_or(Value::Null),
            cookie: cookies.into_iter().next(),
        })
    }

    pub async fn login(&mut self, password: &str) -> Result<(), Failure> {
        self.cookie = None;
        let reply = self
            .call(
                Method::POST,
                "/api/auth/login",
                Some(json!({"account":"connected-operator","password":password})),
            )
            .await?;
        if reply.status != StatusCode::OK || reply.cookie.is_none() {
            return Err(Failure::WrongObservation);
        }
        self.cookie = reply.cookie;
        let me = self.get("/api/auth/me").await?;
        if me["account"] != "connected-operator" || me["role"] != "operator" {
            return Err(Failure::WrongObservation);
        }
        Ok(())
    }

    pub async fn get(&self, path: &str) -> Result<Value, Failure> {
        self.call(Method::GET, path, None).await?.ok()
    }
    pub async fn post(&self, path: &str, body: Value) -> Result<Value, Failure> {
        self.call(Method::POST, path, Some(body)).await?.ok()
    }

    pub async fn denied(
        &self,
        method: Method,
        path: &str,
        body: Option<Value>,
    ) -> Result<(), Failure> {
        let reply = self.call(method, path, body).await?;
        if reply.status == StatusCode::UNAUTHORIZED {
            Ok(())
        } else {
            Err(Failure::WrongObservation)
        }
    }

    pub async fn conflict(&self, path: &str, body: Value, error: &str) -> Result<(), Failure> {
        let reply = self.call(Method::POST, path, Some(body)).await?;
        if reply.status == StatusCode::CONFLICT && reply.value["error"] == error {
            Ok(())
        } else {
            Err(Failure::WrongObservation)
        }
    }

    pub fn recording_client(&self) -> Result<Client, Failure> {
        let mut headers = header::HeaderMap::new();
        headers.insert(
            header::COOKIE,
            self.cookie
                .as_deref()
                .ok_or(Failure::Setup)?
                .parse()
                .map_err(|_| Failure::Setup)?,
        );
        headers.insert(
            header::ORIGIN,
            self.base.parse().map_err(|_| Failure::Setup)?,
        );
        Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .default_headers(headers)
            .timeout(Duration::from_secs(1))
            .build()
            .map_err(|_| Failure::Setup)
    }
}
