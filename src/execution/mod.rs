use crate::{
    market_data::{binance::{BinanceClient, BinanceOrderResponse}, uniswap::{DexExecutionPlan, UniswapClient}},
    routing::compare_routes,
};
use serde::Serialize;

#[derive(Debug, Serialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum BestExecution {
    BinanceExecuted {
        best_venue: String,
        order: BinanceOrderResponse,
    },
    DexRequiresWallet {
        best_venue: String,
        plan: DexExecutionPlan,
    },
}

pub async fn execute_best(
    binance: &BinanceClient,
    uniswap: &UniswapClient,
    input_usdt: f64,
    wallet: Option<&str>,
    live_trading_enabled: bool,
) -> anyhow::Result<BestExecution> {
    let comparison = compare_routes(binance, uniswap, input_usdt).await?;

    match comparison.best_venue.as_str() {
        "BINANCE" => {
            if !live_trading_enabled {
                anyhow::bail!("live Binance trading is disabled; set ENABLE_LIVE_TRADING=true after testing");
            }
            let order = binance.market_buy_eth_with_usdt(input_usdt).await?;
            Ok(BestExecution::BinanceExecuted { best_venue: "BINANCE".into(), order })
        }
        _ => {
            let wallet = wallet.ok_or_else(|| anyhow::anyhow!("wallet address is required for DEX execution"))?;
            let plan = uniswap.build_execution_plan(wallet, &comparison.dex_meta)?;
            Ok(BestExecution::DexRequiresWallet { best_venue: "UNISWAP_V3".into(), plan })
        }
    }
}
