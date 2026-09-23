use crate::market_data::{binance::BinanceClient, uniswap::{UniswapClient, UniswapQuoteMeta}, VenueQuote};
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct RouteComparison {
    pub pair: String,
    pub direction: String,
    pub input_usdt: f64,
    pub cex: VenueQuote,
    pub dex: VenueQuote,
    pub best_venue: String,
    pub difference_eth: f64,
    pub dex_meta: UniswapQuoteMeta,
    pub warning: String,
}

pub async fn compare_routes(
    binance: &BinanceClient,
    uniswap: &UniswapClient,
    input_usdt: f64,
) -> anyhow::Result<RouteComparison> {
    let (cex, (dex, dex_meta)) = tokio::try_join!(
        binance.quote_buy_eth(input_usdt),
        uniswap.quote_buy_eth(input_usdt)
    )?;

    let best_venue = if dex.effective_eth > cex.effective_eth {
        "UNISWAP_V3"
    } else {
        "BINANCE"
    };

    Ok(RouteComparison {
        pair: "USDT/ETH".into(),
        direction: "USDT -> ETH".into(),
        input_usdt,
        difference_eth: (dex.effective_eth - cex.effective_eth).abs(),
        best_venue: best_venue.into(),
        cex,
        dex,
        dex_meta,
        warning: "Comparison assumes funds are already available on the selected venue. Cross-venue transfers/withdrawal fees are not included.".into(),
    })
}
