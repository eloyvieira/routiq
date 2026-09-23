pub mod binance;
pub mod uniswap;

use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct VenueQuote {
    pub venue: String,
    pub input_usdt: f64,
    pub gross_eth: f64,
    pub fee_eth: f64,
    pub gas_eth: f64,
    pub effective_eth: f64,
    pub average_price_usdt_per_eth: f64,
    pub details: String,
}
