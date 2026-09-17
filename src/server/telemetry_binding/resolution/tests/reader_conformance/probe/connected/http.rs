use super::*;
use reqwest::{Client, Method, StatusCode, header};
use serde_json::{Value, json};

pub(in super::super) struct Http {
    base: String,
    client: Client,
    cookie: Option<String>,
    cookie_name: &'static str,
    last: parking_lot::Mutex<Option<HttpObservation>>,
}

pub(in super::super) struct Reply {
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
        Self::with_timeout(address, Duration::from_secs(60))
    }

    pub fn with_timeout(address: std::net::SocketAddr, timeout: Duration) -> Result<Self, Failure> {
        Ok(Self {
            base: format!("http://{address}"),
            cookie: None,
            cookie_name: "cowboy_user=",
            last: parking_lot::Mutex::new(None),
            client: Client::builder()
                .no_proxy()
                .redirect(reqwest::redirect::Policy::none())
                .connect_timeout(Duration::from_secs(1))
                .timeout(timeout)
                .build()
                .map_err(|_| Failure::Setup)?,
        })
    }

    /// Carry the real fixture cookie across a stopped-state copy and new port.
    /// A login response is the only source; it never enters a receipt or log.
    pub fn at(&self, address: std::net::SocketAddr) -> Result<Self, Failure> {
        let mut next = Self::new(address)?;
        next.cookie = Some(self.cookie.clone().ok_or(Failure::Setup)?);
        next.cookie_name = self.cookie_name;
        Ok(next)
    }

    pub async fn call(
        &self,
        method: Method,
        path: &str,
        body: Option<Value>,
    ) -> Result<Reply, Failure> {
        let started = std::time::Instant::now();
        *self.last.lock() = Some(HttpObservation {
            status: None,
            elapsed_ms: 0,
            result: HttpResult::Transport,
        });
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
        let mut response = request.send().await.map_err(|error| {
            if let Some(last) = self.last.lock().as_mut() {
                last.elapsed_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
                last.result = if error.is_timeout() {
                    HttpResult::Timeout
                } else {
                    HttpResult::Transport
                };
            }
            Failure::WrongObservation
        })?;
        let status = response.status();
        *self.last.lock() = Some(HttpObservation {
            status: Some(status.as_u16()),
            elapsed_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
            result: HttpResult::Body,
        });
        if (path.starts_with("/api/telemetry/") || path.starts_with("/api/code/buffer"))
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
            .filter(|h| h.starts_with(self.cookie_name))
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
        let value: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
        let result = if status == StatusCode::UNAUTHORIZED {
            HttpResult::Denied
        } else if value["error"] == "outcome_unverified" {
            HttpResult::OutcomeUnverified
        } else if value["error"] == "preview_or_evidence_changed" {
            HttpResult::Changed
        } else if value["operation"]["phase"] == "needs_attention" {
            HttpResult::NeedsAttention
        } else if status == StatusCode::OK && !value.is_null() {
            HttpResult::Json
        } else {
            HttpResult::Other
        };
        *self.last.lock() = Some(HttpObservation {
            status: Some(status.as_u16()),
            elapsed_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
            result,
        });
        Ok(Reply {
            status,
            value,
            cookie: cookies.into_iter().next(),
        })
    }

    pub fn last(&self) -> Option<HttpObservation> {
        self.last.lock().clone()
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

    /// A separate real fixture Admin login; a Product Operator cookie must not
    /// cross the Catalog's stronger middleware boundary.
    pub async fn catalog_admin(
        address: std::net::SocketAddr,
        password: &str,
    ) -> Result<Self, Failure> {
        let mut http = Self::new(address)?;
        http.cookie_name = "cowboy_admin=";
        let reply = http
            .call(
                Method::POST,
                "/api/admin/auth/login",
                Some(json!({"account":"catalog-operator","password":password})),
            )
            .await?;
        if reply.status != StatusCode::OK
            || reply.cookie.is_none()
            || reply.value["role"] != "operator"
        {
            return Err(Failure::WrongObservation);
        }
        http.cookie = reply.cookie;
        Ok(http)
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
