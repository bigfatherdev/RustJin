use axum::{
    extract::{Path, Query},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::{delete, get, patch, post, put},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    net::SocketAddr,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};
use tower_http::cors::CorsLayer;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

// Constantes de segurança
const MAX_REDIRECTS: u32 = 10;

// Métricas globais (thread-safe)
#[derive(Clone)]
struct Metrics {
    total_requests: Arc<AtomicU64>,
    successful_requests: Arc<AtomicU64>,
    failed_requests: Arc<AtomicU64>,
    redirects_blocked: Arc<AtomicU64>,
    delays_blocked: Arc<AtomicU64>,
    bytes_blocked: Arc<AtomicU64>,
    dangerous_urls_blocked: Arc<AtomicU64>,
    endpoint_stats: Arc<Mutex<HashMap<String, u64>>>,
}

impl Metrics {
    fn new() -> Self {
        Self {
            total_requests: Arc::new(AtomicU64::new(0)),
            successful_requests: Arc::new(AtomicU64::new(0)),
            failed_requests: Arc::new(AtomicU64::new(0)),
            redirects_blocked: Arc::new(AtomicU64::new(0)),
            delays_blocked: Arc::new(AtomicU64::new(0)),
            bytes_blocked: Arc::new(AtomicU64::new(0)),
            dangerous_urls_blocked: Arc::new(AtomicU64::new(0)),
            endpoint_stats: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    fn increment_total(&self) {
        self.total_requests.fetch_add(1, Ordering::Relaxed);
    }

    fn increment_success(&self) {
        self.successful_requests.fetch_add(1, Ordering::Relaxed);
    }

    fn increment_failed(&self) {
        self.failed_requests.fetch_add(1, Ordering::Relaxed);
    }

    fn increment_redirects_blocked(&self) {
        self.redirects_blocked.fetch_add(1, Ordering::Relaxed);
    }

    fn increment_delays_blocked(&self) {
        self.delays_blocked.fetch_add(1, Ordering::Relaxed);
    }

    fn increment_bytes_blocked(&self) {
        self.bytes_blocked.fetch_add(1, Ordering::Relaxed);
    }

    fn increment_dangerous_urls(&self) {
        self.dangerous_urls_blocked.fetch_add(1, Ordering::Relaxed);
    }

    fn record_endpoint(&self, endpoint: String) {
        if let Ok(mut stats) = self.endpoint_stats.lock() {
            *stats.entry(endpoint).or_insert(0) += 1;
        }
    }

    fn get_stats(&self) -> MetricsResponse {
        let endpoint_stats = self
            .endpoint_stats
            .lock()
            .ok()
            .map(|stats| stats.clone())
            .unwrap_or_default();

        MetricsResponse {
            total_requests: self.total_requests.load(Ordering::Relaxed),
            successful_requests: self.successful_requests.load(Ordering::Relaxed),
            failed_requests: self.failed_requests.load(Ordering::Relaxed),
            security_blocks: SecurityBlocks {
                redirects_blocked: self.redirects_blocked.load(Ordering::Relaxed),
                delays_blocked: self.delays_blocked.load(Ordering::Relaxed),
                bytes_blocked: self.bytes_blocked.load(Ordering::Relaxed),
                dangerous_urls_blocked: self.dangerous_urls_blocked.load(Ordering::Relaxed),
            },
            endpoint_stats,
        }
    }
}

#[derive(Serialize)]
struct MetricsResponse {
    total_requests: u64,
    successful_requests: u64,
    failed_requests: u64,
    security_blocks: SecurityBlocks,
    endpoint_stats: HashMap<String, u64>,
}

#[derive(Serialize)]
struct SecurityBlocks {
    redirects_blocked: u64,
    delays_blocked: u64,
    bytes_blocked: u64,
    dangerous_urls_blocked: u64,
}

#[tokio::main]
async fn main() {
    // Inicializa o sistema de logs
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "httpbin_rust=info,tower_http=info".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    let app_state = Arc::new(AppState::new());

    // Configura as rotas
    let app = Router::new()
        // Métodos HTTP básicos
        .route("/get", get(handle_get))
        .route("/post", post(handle_post))
        .route("/put", put(handle_put))
        .route("/patch", patch(handle_patch))
        .route("/delete", delete(handle_delete))
        
        // Informações da requisição
        .route("/headers", get(handle_headers))
        .route("/ip", get(handle_ip))
        .route("/user-agent", get(handle_user_agent))
        
        // Status codes
        .route("/status/:code", get(handle_status))
        .route("/status/:code", post(handle_status))
        .route("/status/:code", put(handle_status))
        .route("/status/:code", delete(handle_status))
        
        // Delays
        .route("/delay/:seconds", get(handle_delay))
        
        // Cookies
        .route("/cookies", get(handle_cookies_get))
        .route("/cookies/set", get(handle_cookies_set))
        .route("/cookies/delete", get(handle_cookies_delete))
        
        // Autenticação
        .route("/basic-auth/:user/:password", get(handle_basic_auth))
        .route("/bearer", get(handle_bearer_auth))
        
        // Redirecionamentos
        .route("/redirect/:n", get(handle_redirect))
        .route("/redirect-to", get(handle_redirect_to))
        .route("/absolute-redirect/:n", get(handle_absolute_redirect))
        
        // Response formats
        .route("/json", get(handle_json))
        .route("/html", get(handle_html))
        .route("/xml", get(handle_xml))
        
        // Imagens
        .route("/image", get(handle_image))
        .route("/image/:format", get(handle_image_format))
        
        // Bytes
        .route("/bytes/:n", get(handle_bytes))
        
        // Stream
        .route("/stream/:n", get(handle_stream))
        
        // UUID
        .route("/uuid", get(handle_uuid))
        
        // Base64
        .route("/base64/:value", get(handle_base64_decode))
        
        // Logo
        .route("/logo.png", get(handle_logo))
        
        // Métricas e status
        .route("/metrics", get(handle_metrics))
        .route("/health", get(handle_health))
        
        // Anything
        .route("/anything", get(handle_anything))
        .route("/anything", post(handle_anything))
        .route("/anything/*path", get(handle_anything))
        .route("/anything/*path", post(handle_anything))
        
        // Home
        .route("/", get(handle_home))
        
        .layer(CorsLayer::permissive())
        .with_state(app_state);

    let addr = SocketAddr::from(([0, 0, 0, 0], 8105));
    tracing::info!("🚀 Servidor RustJin iniciado");
    tracing::info!("📡 Porta: {}", addr.port());
    tracing::info!("🌐 URL: https://rustjin.blackcerb.com.br");
    tracing::info!("📊 Métricas: https://rustjin.blackcerb.com.br/metrics");
    tracing::info!("💚 Health: https://rustjin.blackcerb.com.br/health");

    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    axum::serve(listener, app).await.unwrap();
}

// State compartilhado
#[derive(Clone)]
struct AppState {
    start_time: chrono::DateTime<chrono::Utc>,
    metrics: Metrics,
}

impl AppState {
    fn new() -> Self {
        Self {
            start_time: chrono::Utc::now(),
            metrics: Metrics::new(),
        }
    }
}

// Estruturas de resposta
#[derive(Serialize)]
struct RequestInfo {
    args: HashMap<String, String>,
    headers: HashMap<String, String>,
    origin: String,
    url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    data: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    json: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    form: Option<HashMap<String, String>>,
}

// Função auxiliar para extrair informações da requisição
fn extract_request_info(
    headers: &HeaderMap,
    query: Query<HashMap<String, String>>,
    body: Option<String>,
) -> RequestInfo {
    let headers_map: HashMap<String, String> = headers
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or("").to_string()))
        .collect();

    let origin = headers
        .get("x-forwarded-for")
        .or_else(|| headers.get("x-real-ip"))
        .and_then(|v| v.to_str().ok())
        .unwrap_or("127.0.0.1")
        .to_string();

    let json_data = body.as_ref().and_then(|b| serde_json::from_str(b).ok());

    RequestInfo {
        args: query.0,
        headers: headers_map,
        origin,
        url: "https://rustjin.blackcerb.com.br".to_string(),
        data: body.clone(),
        json: json_data,
        form: None,
    }
}

// Handlers - Métricas e Health

async fn handle_metrics(
    axum::extract::State(state): axum::extract::State<Arc<AppState>>,
) -> impl IntoResponse {
    state.metrics.increment_total();
    state.metrics.record_endpoint("/metrics".to_string());
    
    let stats = state.metrics.get_stats();
    state.metrics.increment_success();
    
    Json(stats)
}

async fn handle_health(
    axum::extract::State(state): axum::extract::State<Arc<AppState>>,
) -> impl IntoResponse {
    state.metrics.increment_total();
    state.metrics.record_endpoint("/health".to_string());
    
    let uptime = chrono::Utc::now()
        .signed_duration_since(state.start_time)
        .num_seconds();
    
    state.metrics.increment_success();
    
    Json(json!({
        "status": "healthy",
        "uptime_seconds": uptime,
        "started_at": state.start_time.to_rfc3339(),
        "version": env!("CARGO_PKG_VERSION"),
        "service": "RustJin"
    }))
}

// Handlers originais

async fn handle_logo(
    axum::extract::State(state): axum::extract::State<Arc<AppState>>,
) -> impl IntoResponse {
    state.metrics.increment_total();
    state.metrics.record_endpoint("/logo.png".to_string());
    
    let logo = include_bytes!("../logo.png");
    state.metrics.increment_success();
    
    (
        StatusCode::OK,
        [("content-type", "image/png")],
        logo.as_slice()
    )
}

async fn handle_home(
    axum::extract::State(state): axum::extract::State<Arc<AppState>>,
) -> impl IntoResponse {
    state.metrics.increment_total();
    state.metrics.record_endpoint("/".to_string());
    
    let html = include_str!("../index.html");
    state.metrics.increment_success();
    
    (StatusCode::OK, [("content-type", "text/html; charset=utf-8")], html)
}

async fn handle_get(
    axum::extract::State(state): axum::extract::State<Arc<AppState>>,
    headers: HeaderMap,
    query: Query<HashMap<String, String>>,
) -> impl IntoResponse {
    state.metrics.increment_total();
    state.metrics.record_endpoint("/get".to_string());
    state.metrics.increment_success();
    
    Json(extract_request_info(&headers, query, None))
}

async fn handle_post(
    axum::extract::State(state): axum::extract::State<Arc<AppState>>,
    headers: HeaderMap,
    query: Query<HashMap<String, String>>,
    body: String,
) -> impl IntoResponse {
    state.metrics.increment_total();
    state.metrics.record_endpoint("/post".to_string());
    state.metrics.increment_success();
    
    Json(extract_request_info(&headers, query, Some(body)))
}

async fn handle_put(
    axum::extract::State(state): axum::extract::State<Arc<AppState>>,
    headers: HeaderMap,
    query: Query<HashMap<String, String>>,
    body: String,
) -> impl IntoResponse {
    state.metrics.increment_total();
    state.metrics.record_endpoint("/put".to_string());
    state.metrics.increment_success();
    
    Json(extract_request_info(&headers, query, Some(body)))
}

async fn handle_patch(
    axum::extract::State(state): axum::extract::State<Arc<AppState>>,
    headers: HeaderMap,
    query: Query<HashMap<String, String>>,
    body: String,
) -> impl IntoResponse {
    state.metrics.increment_total();
    state.metrics.record_endpoint("/patch".to_string());
    state.metrics.increment_success();
    
    Json(extract_request_info(&headers, query, Some(body)))
}

async fn handle_delete(
    axum::extract::State(state): axum::extract::State<Arc<AppState>>,
    headers: HeaderMap,
    query: Query<HashMap<String, String>>,
) -> impl IntoResponse {
    state.metrics.increment_total();
    state.metrics.record_endpoint("/delete".to_string());
    state.metrics.increment_success();
    
    Json(extract_request_info(&headers, query, None))
}

async fn handle_headers(
    axum::extract::State(state): axum::extract::State<Arc<AppState>>,
    headers: HeaderMap,
) -> impl IntoResponse {
    state.metrics.increment_total();
    state.metrics.record_endpoint("/headers".to_string());
    
    let headers_map: HashMap<String, String> = headers
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or("").to_string()))
        .collect();
    
    state.metrics.increment_success();
    Json(json!({ "headers": headers_map }))
}

async fn handle_ip(
    axum::extract::State(state): axum::extract::State<Arc<AppState>>,
    headers: HeaderMap,
) -> impl IntoResponse {
    state.metrics.increment_total();
    state.metrics.record_endpoint("/ip".to_string());
    
    let origin = headers
        .get("x-forwarded-for")
        .or_else(|| headers.get("x-real-ip"))
        .and_then(|v| v.to_str().ok())
        .unwrap_or("127.0.0.1");
    
    state.metrics.increment_success();
    Json(json!({ "origin": origin }))
}

async fn handle_user_agent(
    axum::extract::State(state): axum::extract::State<Arc<AppState>>,
    headers: HeaderMap,
) -> impl IntoResponse {
    state.metrics.increment_total();
    state.metrics.record_endpoint("/user-agent".to_string());
    
    let user_agent = headers
        .get("user-agent")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("Unknown");
    
    state.metrics.increment_success();
    Json(json!({ "user-agent": user_agent }))
}

async fn handle_status(
    axum::extract::State(state): axum::extract::State<Arc<AppState>>,
    Path(code): Path<u16>,
) -> impl IntoResponse {
    state.metrics.increment_total();
    state.metrics.record_endpoint(format!("/status/{}", code));
    
    let status = StatusCode::from_u16(code).unwrap_or(StatusCode::OK);
    
    if status.is_success() {
        state.metrics.increment_success();
    } else {
        state.metrics.increment_failed();
    }
    
    (status, "")
}

async fn handle_delay(
    axum::extract::State(state): axum::extract::State<Arc<AppState>>,
    Path(seconds): Path<u64>,
) -> impl IntoResponse {
    state.metrics.increment_total();
    state.metrics.record_endpoint(format!("/delay/{}", seconds));
    
    const MAX_DELAY: u64 = 10;
    
    if seconds > MAX_DELAY {
        state.metrics.increment_delays_blocked();
        state.metrics.increment_failed();
        
        tracing::warn!("⚠️  Delay bloqueado: {} segundos (max: {})", seconds, MAX_DELAY);
        
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({
                "error": "Delay too long",
                "max_delay": MAX_DELAY,
                "requested": seconds,
                "message": format!("Maximum delay is {} seconds", MAX_DELAY)
            }))
        ).into_response();
    }
    
    tracing::info!("⏳ Delay de {} segundos iniciado", seconds);
    tokio::time::sleep(Duration::from_secs(seconds)).await;
    
    state.metrics.increment_success();
    
    Json(json!({
        "delay": seconds,
        "message": format!("Delayed for {} seconds", seconds)
    })).into_response()
}

async fn handle_cookies_get(
    axum::extract::State(state): axum::extract::State<Arc<AppState>>,
    headers: HeaderMap,
) -> impl IntoResponse {
    state.metrics.increment_total();
    state.metrics.record_endpoint("/cookies".to_string());
    
    let cookies = headers
        .get("cookie")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    
    let cookies_map: HashMap<String, String> = cookies
        .split(';')
        .filter_map(|c| {
            let parts: Vec<&str> = c.trim().splitn(2, '=').collect();
            if parts.len() == 2 {
                Some((parts[0].to_string(), parts[1].to_string()))
            } else {
                None
            }
        })
        .collect();
    
    state.metrics.increment_success();
    Json(json!({ "cookies": cookies_map }))
}

#[derive(Deserialize)]
struct CookieParams {
    #[serde(flatten)]
    cookies: HashMap<String, String>,
}

async fn handle_cookies_set(
    axum::extract::State(state): axum::extract::State<Arc<AppState>>,
    Query(params): Query<CookieParams>,
) -> impl IntoResponse {
    state.metrics.increment_total();
    state.metrics.record_endpoint("/cookies/set".to_string());
    
    let cookie_headers: Vec<_> = params
        .cookies
        .iter()
        .map(|(k, v)| format!("{}={}", k, v))
        .collect();
    
    let mut headers = HeaderMap::new();
    for cookie in cookie_headers {
        headers.append("set-cookie", cookie.parse().unwrap());
    }
    
    state.metrics.increment_success();
    (headers, Json(json!({ "cookies": params.cookies })))
}

async fn handle_cookies_delete(
    axum::extract::State(state): axum::extract::State<Arc<AppState>>,
    Query(params): Query<HashMap<String, String>>,
) -> impl IntoResponse {
    state.metrics.increment_total();
    state.metrics.record_endpoint("/cookies/delete".to_string());
    
    let mut headers = HeaderMap::new();
    
    if let Some(name) = params.get("name") {
        let cookie = format!("{}=; Max-Age=0", name);
        headers.append("set-cookie", cookie.parse().unwrap());
    }
    
    state.metrics.increment_success();
    (headers, Json(json!({ "message": "Cookie deleted" })))
}

async fn handle_basic_auth(
    axum::extract::State(state): axum::extract::State<Arc<AppState>>,
    Path((user, password)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<Json<Value>, StatusCode> {
    use base64::{Engine as _, engine::general_purpose};
    
    state.metrics.increment_total();
    state.metrics.record_endpoint(format!("/basic-auth/{}/{}", user, "***"));
    
    if let Some(auth_header) = headers.get("authorization") {
        if let Ok(auth_str) = auth_header.to_str() {
            if auth_str.starts_with("Basic ") {
                if let Ok(decoded) = general_purpose::STANDARD.decode(&auth_str[6..]) {
                    if let Ok(credentials) = String::from_utf8(decoded) {
                        let parts: Vec<&str> = credentials.splitn(2, ':').collect();
                        if parts.len() == 2 && parts[0] == user && parts[1] == password {
                            state.metrics.increment_success();
                            tracing::info!("✅ Autenticação básica bem-sucedida para: {}", user);
                            return Ok(Json(json!({
                                "authenticated": true,
                                "user": user
                            })));
                        }
                    }
                }
            }
        }
    }
    
    state.metrics.increment_failed();
    tracing::warn!("❌ Falha na autenticação básica para: {}", user);
    Err(StatusCode::UNAUTHORIZED)
}

async fn handle_bearer_auth(
    axum::extract::State(state): axum::extract::State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<Value>, StatusCode> {
    state.metrics.increment_total();
    state.metrics.record_endpoint("/bearer".to_string());
    
    if let Some(auth_header) = headers.get("authorization") {
        if let Ok(auth_str) = auth_header.to_str() {
            if auth_str.starts_with("Bearer ") {
                let token = &auth_str[7..];
                state.metrics.increment_success();
                tracing::info!("✅ Autenticação bearer bem-sucedida");
                return Ok(Json(json!({
                    "authenticated": true,
                    "token": token
                })));
            }
        }
    }
    
    state.metrics.increment_failed();
    tracing::warn!("❌ Falha na autenticação bearer");
    Err(StatusCode::UNAUTHORIZED)
}

async fn handle_redirect(
    axum::extract::State(state): axum::extract::State<Arc<AppState>>,
    Path(n): Path<u32>,
) -> impl IntoResponse {
    state.metrics.increment_total();
    state.metrics.record_endpoint(format!("/redirect/{}", n));
    
    if n > MAX_REDIRECTS {
        state.metrics.increment_redirects_blocked();
        state.metrics.increment_failed();
        
        tracing::warn!("🚫 Redirecionamento bloqueado: {} (max: {})", n, MAX_REDIRECTS);
        
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({
                "error": "Too many redirects",
                "max_allowed": MAX_REDIRECTS,
                "requested": n,
                "message": format!("Maximum {} redirects allowed", MAX_REDIRECTS)
            }))
        ).into_response();
    }
    
    if n <= 1 {
        state.metrics.increment_success();
        return (
            StatusCode::OK,
            [("location", "/get".to_string())],
            ""
        ).into_response();
    }
    
    state.metrics.increment_success();
    let location = format!("/redirect/{}", n - 1);
    (
        StatusCode::FOUND,
        [("location", location)],
        "",
    ).into_response()
}

#[derive(Deserialize)]
struct RedirectToParams {
    url: String,
}

async fn handle_redirect_to(
    axum::extract::State(state): axum::extract::State<Arc<AppState>>,
    Query(params): Query<RedirectToParams>,
) -> impl IntoResponse {
    state.metrics.increment_total();
    state.metrics.record_endpoint("/redirect-to".to_string());
    
    let url = params.url.trim();
    
    let dangerous_protocols = ["javascript:", "data:", "file:", "vbscript:"];
    let url_lower = url.to_lowercase();
    
    for protocol in &dangerous_protocols {
        if url_lower.starts_with(protocol) {
            state.metrics.increment_dangerous_urls();
            state.metrics.increment_failed();
            
            tracing::warn!("🚨 URL perigosa bloqueada: {} (protocolo: {})", url, protocol);
            
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({
                    "error": "Invalid protocol",
                    "message": "Protocol not allowed for security reasons"
                }))
            ).into_response();
        }
    }
    
    if url.len() > 2048 {
        state.metrics.increment_failed();
        
        tracing::warn!("⚠️  URL muito longa bloqueada: {} caracteres", url.len());
        
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({
                "error": "URL too long",
                "max_length": 2048,
                "message": "URL exceeds maximum allowed length"
            }))
        ).into_response();
    }
    
    let final_url = if !url.starts_with("http://") && !url.starts_with("https://") {
        format!("http://{}", url)
    } else {
        url.to_string()
    };
    
    state.metrics.increment_success();
    tracing::info!("↪️  Redirecionando para: {}", final_url);
    
    (
        StatusCode::FOUND,
        [("location", final_url)],
        ""
    ).into_response()
}

async fn handle_absolute_redirect(
    axum::extract::State(state): axum::extract::State<Arc<AppState>>,
    Path(n): Path<u32>,
) -> impl IntoResponse {
    state.metrics.increment_total();
    state.metrics.record_endpoint(format!("/absolute-redirect/{}", n));
    
    if n > MAX_REDIRECTS {
        state.metrics.increment_redirects_blocked();
        state.metrics.increment_failed();
        
        tracing::warn!("🚫 Redirecionamento absoluto bloqueado: {} (max: {})", n, MAX_REDIRECTS);
        
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({
                "error": "Too many redirects",
                "max_allowed": MAX_REDIRECTS,
                "requested": n,
                "message": format!("Maximum {} redirects allowed", MAX_REDIRECTS)
            }))
        ).into_response();
    }
    
    if n <= 1 {
        state.metrics.increment_success();
        return (
            StatusCode::OK,
            [("location", "https://rustjin.blackcerb.com.br/get".to_string())],
            "",
        ).into_response();
    }
    
    state.metrics.increment_success();
    let location = format!("https://rustjin.blackcerb.com.br/absolute-redirect/{}", n - 1);
    (
        StatusCode::FOUND,
        [("location", location)],
        ""
    ).into_response()
}

async fn handle_json(
    axum::extract::State(state): axum::extract::State<Arc<AppState>>,
) -> impl IntoResponse {
    state.metrics.increment_total();
    state.metrics.record_endpoint("/json".to_string());
    state.metrics.increment_success();
    
    Json(json!({
        "slideshow": {
            "author": "Yours Truly",
            "date": "date of publication",
            "slides": [
                {
                    "title": "Wake up to WonderWidgets!",
                    "type": "all"
                },
                {
                    "items": [
                        "Why <em>WonderWidgets</em> are great",
                        "Who <em>buys</em> WonderWidgets"
                    ],
                    "title": "Overview",
                    "type": "all"
                }
            ],
            "title": "Sample Slide Show"
        }
    }))
}

async fn handle_html(
    axum::extract::State(state): axum::extract::State<Arc<AppState>>,
) -> impl IntoResponse {
    state.metrics.increment_total();
    state.metrics.record_endpoint("/html".to_string());
    state.metrics.increment_success();
    
    let html = r#"<!DOCTYPE html>
<html>
<head>
    <title>HTTPBin HTML</title>
</head>
<body>
    <h1>Herman Melville - Moby-Dick</h1>
    <p>Call me Ishmael. Some years ago...</p>
</body>
</html>"#;
    
    (StatusCode::OK, [("content-type", "text/html")], html)
}

async fn handle_xml(
    axum::extract::State(state): axum::extract::State<Arc<AppState>>,
) -> impl IntoResponse {
    state.metrics.increment_total();
    state.metrics.record_endpoint("/xml".to_string());
    state.metrics.increment_success();
    
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<slideshow>
    <title>Sample Slide Show</title>
    <author>Yours Truly</author>
    <slide>
        <title>Wake up to WonderWidgets!</title>
    </slide>
</slideshow>"#;
    
    (StatusCode::OK, [("content-type", "application/xml")], xml)
}

async fn handle_image(
    axum::extract::State(state): axum::extract::State<Arc<AppState>>,
) -> impl IntoResponse {
    state.metrics.increment_total();
    state.metrics.record_endpoint("/image".to_string());
    state.metrics.increment_success();
    
    let svg = r##"<svg width="200" height="200" xmlns="http://www.w3.org/2000/svg">
        <rect width="200" height="200" fill="#3498db"/>
        <text x="50%" y="50%" text-anchor="middle" fill="white" font-size="20">HTTPBin</text>
    </svg>"##;
    
    (StatusCode::OK, [("content-type", "image/svg+xml")], svg)
}

async fn handle_image_format(
    axum::extract::State(state): axum::extract::State<Arc<AppState>>,
    Path(_format): Path<String>,
) -> impl IntoResponse {
    handle_image(axum::extract::State(state)).await
}

async fn handle_bytes(
    axum::extract::State(state): axum::extract::State<Arc<AppState>>,
    Path(n): Path<usize>,
) -> impl IntoResponse {
    state.metrics.increment_total();
    state.metrics.record_endpoint(format!("/bytes/{}", n));
    
    const MAX_BYTES: usize = 100_000;
    
    if n > MAX_BYTES {
        state.metrics.increment_bytes_blocked();
        state.metrics.increment_failed();
        
        tracing::warn!("🚫 Requisição de bytes bloqueada: {} (max: {})", n, MAX_BYTES);
        
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({
                "error": "Too many bytes requested",
                "max_bytes": MAX_BYTES,
                "requested": n,
                "message": format!("Maximum {} bytes allowed", MAX_BYTES)
            }))
        ).into_response();
    }
    
    let bytes: Vec<u8> = (0..n).map(|i| (i % 256) as u8).collect();
    state.metrics.increment_success();
    
    (
        StatusCode::OK,
        [("content-type", "application/octet-stream")],
        bytes
    ).into_response()
}

async fn handle_stream(
    axum::extract::State(state): axum::extract::State<Arc<AppState>>,
    Path(n): Path<usize>,
) -> impl IntoResponse {
    state.metrics.increment_total();
    state.metrics.record_endpoint(format!("/stream/{}", n));
    
    const MAX_LINES: usize = 100;
    
    if n > MAX_LINES {
        state.metrics.increment_failed();
        
        tracing::warn!("🚫 Requisição de stream bloqueada: {} linhas (max: {})", n, MAX_LINES);
        
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({
                "error": "Too many lines requested",
                "max_lines": MAX_LINES,
                "requested": n,
                "message": format!("Maximum {} lines allowed", MAX_LINES)
            }))
        ).into_response();
    }
    
    let mut lines = Vec::new();
    
    for i in 0..n {
        lines.push(json!({
            "id": i,
            "url": format!("https://rustjin.blackcerb.com.br/stream/{}", n),
            "args": {}
        }).to_string());
    }
    
    state.metrics.increment_success();
    
    (
        StatusCode::OK,
        [("content-type", "application/json")],
        lines.join("\n"),
    ).into_response()
}

async fn handle_uuid(
    axum::extract::State(state): axum::extract::State<Arc<AppState>>,
) -> impl IntoResponse {
    state.metrics.increment_total();
    state.metrics.record_endpoint("/uuid".to_string());
    state.metrics.increment_success();
    
    Json(json!({
        "uuid": uuid::Uuid::new_v4().to_string()
    }))
}

async fn handle_base64_decode(
    axum::extract::State(state): axum::extract::State<Arc<AppState>>,
    Path(value): Path<String>,
) -> impl IntoResponse {
    use base64::{Engine as _, engine::general_purpose};
    
    state.metrics.increment_total();
    state.metrics.record_endpoint("/base64/decode".to_string());
    
    match general_purpose::STANDARD.decode(&value) {
        Ok(decoded) => match String::from_utf8(decoded) {
            Ok(text) => {
                state.metrics.increment_success();
                (StatusCode::OK, text).into_response()
            },
            Err(_) => {
                state.metrics.increment_failed();
                (StatusCode::BAD_REQUEST, "Invalid UTF-8").into_response()
            },
        },
        Err(_) => {
            state.metrics.increment_failed();
            (StatusCode::BAD_REQUEST, "Invalid base64").into_response()
        },
    }
}

async fn handle_anything(
    axum::extract::State(state): axum::extract::State<Arc<AppState>>,
    headers: HeaderMap,
    query: Query<HashMap<String, String>>,
    body: Option<String>,
) -> impl IntoResponse {
    state.metrics.increment_total();
    state.metrics.record_endpoint("/anything".to_string());
    state.metrics.increment_success();
    
    Json(extract_request_info(&headers, query, body))
}