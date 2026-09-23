mod execution;
mod market_data;
mod portfolio;
mod risk;
mod routing;

use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use execution::{execute_best, BestExecution};
use market_data::{binance::BinanceClient, uniswap::UniswapClient};
use routing::{compare_routes, RouteComparison};
use serde::Deserialize;
use std::{env, net::SocketAddr, sync::Arc};
use tower_http::{cors::CorsLayer, services::ServeDir, trace::TraceLayer};

#[derive(Clone)]
struct AppState {
    binance: BinanceClient,
    uniswap: UniswapClient,
    live_trading_enabled: bool,
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let host = env::var("APP_HOST").unwrap_or_else(|_| "0.0.0.0".into());
    let port: u16 = env::var("APP_PORT").ok().and_then(|v| v.parse().ok()).unwrap_or(8080);

    let binance = BinanceClient::from_env();
    binance.start_ws();

    let state = Arc::new(AppState {
        binance,
        uniswap: UniswapClient::from_env(),
        live_trading_enabled: env_bool("ENABLE_LIVE_TRADING", false),
    });

    let app = Router::new()
        .route("/api/health", get(health))
        .route("/api/quote", get(get_quote))
        .route("/api/execute-best", post(post_execute_best))
        .fallback_service(
            ServeDir::new("web")
                .append_index_html_on_directories(true)
        )
        .layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http())
        .with_state(state);

    let addr: SocketAddr = format!("{host}:{port}").parse().expect("invalid bind address");
    tracing::info!("server running on http://{addr}");
    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    axum::serve(listener, app).await.unwrap();
}

fn env_bool(name: &str, default: bool) -> bool {
    env::var(name)
        .ok()
        .map(|v| matches!(v.to_lowercase().as_str(), "1" | "true" | "yes" | "on"))
        .unwrap_or(default)
}

async fn health(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let book = state.binance.book.read().await;
    Json(serde_json::json!({
        "status": "ok",
        "binance_ws_connected": book.connected,
        "binance_last_update_id": book.last_update_id,
        "binance_ask_levels": book.asks.len(),
        "live_trading_enabled": state.live_trading_enabled
    }))
}

#[derive(Deserialize)]
struct QuoteQuery {
    amount_usdt: Option<f64>
}

async fn get_quote(
    State(state): State<Arc<AppState>>,
    Query(q): Query<QuoteQuery>,
) -> Result<Json<RouteComparison>, (StatusCode, String)> {
    let input = q.amount_usdt.unwrap_or(100.0);
    validate_input(input)?;
    compare_routes(&state.binance, &state.uniswap, input)
        .await
        .map(Json)
        .map_err(internal_bad_gateway)
}

#[derive(Deserialize)]
struct ExecuteRequest {
    amount_usdt: f64,
    wallet: Option<String>,
    confirm: bool,
}

async fn post_execute_best(
    State(state): State<Arc<AppState>>,
    Json(req): Json<ExecuteRequest>,
) -> Result<Json<BestExecution>, (StatusCode, String)> {
    validate_input(req.amount_usdt)?;
    if !req.confirm {
        return Err((StatusCode::BAD_REQUEST, "confirm=true is required".into()));
    }

    execute_best(
        &state.binance,
        &state.uniswap,
        req.amount_usdt,
        req.wallet.as_deref(),
        state.live_trading_enabled,
    )
    .await
    .map(Json)
    .map_err(|e| (StatusCode::BAD_GATEWAY, e.to_string()))
}

fn validate_input(v: f64) -> Result<(), (StatusCode, String)> {
    if !v.is_finite() || v <= 0.0 {
        return Err((StatusCode::BAD_REQUEST, "amount_usdt must be > 0".into()));
    }
    if v > 1_000_000.0 {
        return Err((StatusCode::BAD_REQUEST, "amount_usdt exceeds lab safety limit".into()));
    }
    Ok(())
}

fn internal_bad_gateway(e: anyhow::Error) -> (StatusCode, String) {
    (StatusCode::BAD_GATEWAY, e.to_string())
}
