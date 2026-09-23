use super::VenueQuote;
use anyhow::Context;
use futures_util::StreamExt;
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use std::{env, sync::Arc, time::Duration};
use tokio::sync::RwLock;
use tokio_tungstenite::connect_async;
use url::form_urlencoded;

#[derive(Debug, Clone, Default)]
pub struct BinanceBook {
    pub asks: Vec<(f64, f64)>,
    pub last_update_id: u64,
    pub connected: bool,
}

#[derive(Clone)]
pub struct BinanceClient {
    http: reqwest::Client,
    rest_base: String,
    ws_url: String,
    api_key: Option<String>,
    api_secret: Option<String>,
    taker_fee_rate: f64,
    pub book: Arc<RwLock<BinanceBook>>,
}

#[derive(Debug, Deserialize)]
struct PartialDepth {
    #[serde(rename = "lastUpdateId")]
    last_update_id: u64,
    asks: Vec<[String; 2]>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct BinanceOrderResponse {
    #[serde(rename = "symbol")]
    pub symbol: String,
    #[serde(rename = "orderId")]
    pub order_id: u64,
    #[serde(rename = "status")]
    pub status: String,
    #[serde(rename = "executedQty", default)]
    pub executed_qty: String,
    #[serde(rename = "cummulativeQuoteQty", default)]
    pub cummulative_quote_qty: String,
}

impl BinanceClient {
    pub fn from_env() -> Self {
        let taker_fee_rate = env::var("BINANCE_TAKER_FEE_RATE")
            .ok()
            .and_then(|x| x.parse().ok())
            .unwrap_or(0.001);

        Self {
            http: reqwest::Client::new(),
            rest_base: env::var("BINANCE_BASE_URL").unwrap_or_else(|_| "https://api.binance.com".into()),
            ws_url: env::var("BINANCE_WS_URL")
                .unwrap_or_else(|_| "wss://stream.binance.com:9443/ws/ethusdt@depth20@100ms".into()),
            api_key: env::var("BINANCE_API_KEY").ok(),
            api_secret: env::var("BINANCE_API_SECRET").ok(),
            taker_fee_rate,
            book: Arc::new(RwLock::new(BinanceBook::default())),
        }
    }

    pub fn start_ws(&self) {
        let url = self.ws_url.clone();
        let book = self.book.clone();
        tokio::spawn(async move {
            loop {
                tracing::info!(%url, "connecting Binance market-data websocket");
                match connect_async(&url).await {
                    Ok((stream, _)) => {
                        {
                            let mut b = book.write().await;
                            b.connected = true;
                        }
                        let (_, mut read) = stream.split();
                        while let Some(msg) = read.next().await {
                            match msg {
                                Ok(m) if m.is_text() => {
                                    if let Ok(depth) = serde_json::from_str::<PartialDepth>(m.to_text().unwrap_or("")) {
                                        let mut asks = Vec::with_capacity(depth.asks.len());
                                        for [p, q] in depth.asks {
                                            if let (Ok(p), Ok(q)) = (p.parse::<f64>(), q.parse::<f64>()) {
                                                asks.push((p, q));
                                            }
                                        }
                                        asks.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
                                        let mut b = book.write().await;
                                        b.asks = asks;
                                        b.last_update_id = depth.last_update_id;
                                    }
                                }
                                Ok(_) => {}
                                Err(e) => {
                                    tracing::warn!(error=%e, "Binance websocket read failed");
                                    break;
                                }
                            }
                        }
                    }
                    Err(e) => tracing::warn!(error=%e, "Binance websocket connect failed"),
                }
                {
                    let mut b = book.write().await;
                    b.connected = false;
                }
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
        });
    }

    pub async fn quote_buy_eth(&self, input_usdt: f64) -> anyhow::Result<VenueQuote> {
        let b = self.book.read().await.clone();
        if b.asks.is_empty() {
            anyhow::bail!("Binance websocket order book is not ready yet");
        }

        let mut remaining_usdt = input_usdt;
        let mut gross_eth = 0.0;
        let mut spent = 0.0;

        for (price, qty_eth) in b.asks {
            let level_cost = price * qty_eth;
            let spend_here = remaining_usdt.min(level_cost);
            let eth_here = spend_here / price;
            gross_eth += eth_here;
            spent += spend_here;
            remaining_usdt -= spend_here;
            if remaining_usdt <= 0.00000001 {
                break;
            }
        }

        if remaining_usdt > 0.01 || gross_eth <= 0.0 {
            anyhow::bail!("insufficient top-20 Binance depth for {:.2} USDT", input_usdt);
        }

        // Conservative comparison: assume the taker fee effectively reduces received ETH.
        let fee_eth = gross_eth * self.taker_fee_rate;
        let effective_eth = gross_eth - fee_eth;
        let avg = spent / gross_eth;

        Ok(VenueQuote {
            venue: "BINANCE".into(),
            input_usdt,
            gross_eth,
            fee_eth,
            gas_eth: 0.0,
            effective_eth,
            average_price_usdt_per_eth: avg,
            details: format!(
                "Binance Spot partial depth WS (top 20), taker fee configured at {:.4}%",
                self.taker_fee_rate * 100.0
            ),
        })
    }

    pub async fn market_buy_eth_with_usdt(&self, input_usdt: f64) -> anyhow::Result<BinanceOrderResponse> {
        let api_key = self.api_key.as_deref().context("BINANCE_API_KEY is not configured")?;
        let secret = self.api_secret.as_deref().context("BINANCE_API_SECRET is not configured")?;
        let timestamp = chrono::Utc::now().timestamp_millis().to_string();

        // Build the percent-encoded payload first, then sign exactly that payload.
        let payload = form_urlencoded::Serializer::new(String::new())
            .append_pair("symbol", "ETHUSDT")
            .append_pair("side", "BUY")
            .append_pair("type", "MARKET")
            .append_pair("quoteOrderQty", &format!("{input_usdt:.6}"))
            .append_pair("newOrderRespType", "RESULT")
            .append_pair("recvWindow", "5000")
            .append_pair("timestamp", &timestamp)
            .finish();

        let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes())?;
        mac.update(payload.as_bytes());
        let signature = hex::encode(mac.finalize().into_bytes());

        let url = format!(
            "{}/api/v3/order?{}&signature={}",
            self.rest_base.trim_end_matches('/'),
            payload,
            signature
        );

        let res = self
            .http
            .post(url)
            .header("X-MBX-APIKEY", api_key)
            .send()
            .await?;

        if !res.status().is_success() {
            let status = res.status();
            let body = res.text().await.unwrap_or_default();
            anyhow::bail!("Binance order failed ({status}): {body}");
        }

        Ok(res.json::<BinanceOrderResponse>().await?)
    }
}
