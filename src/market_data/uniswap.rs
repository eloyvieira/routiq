use super::VenueQuote;
use anyhow::Context;
use serde::{Deserialize, Serialize};
use sha3::{Digest, Keccak256};
use std::env;

pub const MAINNET_USDT: &str = "0xdAC17F958D2ee523a2206206994597C13D831ec7";
pub const MAINNET_WETH: &str = "0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2";
pub const MAINNET_QUOTER_V2: &str = "0x61fFE014bA17989E743c5F6cB21bF9697530B21e";
pub const MAINNET_SWAP_ROUTER_02: &str = "0x68b3465833fb72A70ecDF485E0e4C7bD8665Fc45";

#[derive(Clone)]
pub struct UniswapClient {
    http: reqwest::Client,
    rpc_url: String,
    quoter: String,
    router: String,
    slippage_bps: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct UniswapQuoteMeta {
    pub fee_tier: u32,
    pub amount_in_raw: String,
    pub amount_out_raw: String,
    pub amount_out_min_raw: String,
    pub gas_estimate: u128,
    pub gas_price_wei: u128,
}

#[derive(Debug, Clone, Serialize)]
pub struct DexExecutionPlan {
    pub chain_id_hex: String,
    pub token_in: String,
    pub token_out: String,
    pub router: String,
    pub amount_in_raw: String,
    pub amount_out_min_raw: String,
    pub fee_tier: u32,
    pub allowance_data: String,
    pub approve_zero_data: String,
    pub approve_amount_data: String,
    pub swap_data: String,
    pub value: String,
}

#[derive(Debug, Deserialize)]
struct RpcEnvelope<T> {
    result: Option<T>,
    error: Option<serde_json::Value>,
}

impl UniswapClient {
    pub fn from_env() -> Self {
        Self {
            http: reqwest::Client::new(),
            rpc_url: env::var("ETH_RPC_URL").unwrap_or_else(|_| "https://ethereum-rpc.publicnode.com".into()),
            quoter: env::var("UNISWAP_QUOTER_V2").unwrap_or_else(|_| MAINNET_QUOTER_V2.into()),
            router: env::var("UNISWAP_SWAP_ROUTER_02").unwrap_or_else(|_| MAINNET_SWAP_ROUTER_02.into()),
            slippage_bps: env::var("DEX_SLIPPAGE_BPS").ok().and_then(|x| x.parse().ok()).unwrap_or(50),
        }
    }

    pub async fn quote_buy_eth(&self, input_usdt: f64) -> anyhow::Result<(VenueQuote, UniswapQuoteMeta)> {
        if input_usdt <= 0.0 {
            anyhow::bail!("input_usdt must be > 0");
        }
        let amount_in = decimal_to_raw(input_usdt, 6)?;
        let gas_price = self.eth_gas_price().await.unwrap_or(0);

        let mut best: Option<(u32, u128, u128)> = None; // fee, amountOut, gas
        for fee in [100u32, 500, 3000, 10000] {
            if let Ok((amount_out, gas_estimate)) = self.quote_exact_input_single(amount_in, fee).await {
                if amount_out > 0 && best.as_ref().map(|(_, out, _)| amount_out > *out).unwrap_or(true) {
                    best = Some((fee, amount_out, gas_estimate));
                }
            }
        }

        let (fee_tier, amount_out_raw, gas_estimate) = best.context("no usable Uniswap V3 USDT/WETH quote found")?;
        let gross_eth = raw_to_decimal(amount_out_raw, 18);
        let gas_eth = (gas_estimate as f64) * (gas_price as f64) / 1e18;
        let effective_eth = (gross_eth - gas_eth).max(0.0);
        let avg = if gross_eth > 0.0 { input_usdt / gross_eth } else { f64::INFINITY };
        let amount_out_min = amount_out_raw.saturating_mul((10_000u128).saturating_sub(self.slippage_bps as u128)) / 10_000u128;

        let quote = VenueQuote {
            venue: "UNISWAP_V3".into(),
            input_usdt,
            gross_eth,
            fee_eth: 0.0, // pool fee is already reflected by QuoterV2 output
            gas_eth,
            effective_eth,
            average_price_usdt_per_eth: avg,
            details: format!(
                "Uniswap V3 QuoterV2 on Ethereum mainnet; selected fee tier {} bps; gas estimate included in effective ETH",
                fee_tier
            ),
        };

        let meta = UniswapQuoteMeta {
            fee_tier,
            amount_in_raw: amount_in.to_string(),
            amount_out_raw: amount_out_raw.to_string(),
            amount_out_min_raw: amount_out_min.to_string(),
            gas_estimate,
            gas_price_wei: gas_price,
        };
        Ok((quote, meta))
    }

    pub fn build_execution_plan(&self, recipient: &str, meta: &UniswapQuoteMeta) -> anyhow::Result<DexExecutionPlan> {
        let recipient = normalize_address(recipient)?;
        let amount_in: u128 = meta.amount_in_raw.parse()?;
        let amount_out_min: u128 = meta.amount_out_min_raw.parse()?;

        let allowance_data = encode_allowance(&recipient, &self.router)?;
        let approve_zero_data = encode_approve(&self.router, 0)?;
        let approve_amount_data = encode_approve(&self.router, amount_in)?;
        let swap_data = encode_exact_input_single(
            MAINNET_USDT,
            MAINNET_WETH,
            meta.fee_tier,
            &recipient,
            amount_in,
            amount_out_min,
        )?;

        Ok(DexExecutionPlan {
            chain_id_hex: "0x1".into(),
            token_in: MAINNET_USDT.into(),
            token_out: MAINNET_WETH.into(),
            router: self.router.clone(),
            amount_in_raw: meta.amount_in_raw.clone(),
            amount_out_min_raw: meta.amount_out_min_raw.clone(),
            fee_tier: meta.fee_tier,
            allowance_data,
            approve_zero_data,
            approve_amount_data,
            swap_data,
            value: "0x0".into(),
        })
    }

    async fn quote_exact_input_single(&self, amount_in: u128, fee: u32) -> anyhow::Result<(u128, u128)> {
        let selector = selector("quoteExactInputSingle((address,address,uint256,uint24,uint160))");
        let mut data = selector;
        data.extend(word_address(MAINNET_USDT)?);
        data.extend(word_address(MAINNET_WETH)?);
        data.extend(word_u128(amount_in));
        data.extend(word_u128(fee as u128));
        data.extend(word_u128(0));

        let result = self.eth_call(&self.quoter, &format!("0x{}", hex::encode(data))).await?;
        let bytes = hex::decode(result.trim_start_matches("0x"))?;
        if bytes.len() < 128 {
            anyhow::bail!("unexpected QuoterV2 response length: {}", bytes.len());
        }
        let amount_out = word_to_u128(&bytes[0..32])?;
        let gas_estimate = word_to_u128(&bytes[96..128])?;
        Ok((amount_out, gas_estimate))
    }

    async fn eth_call(&self, to: &str, data: &str) -> anyhow::Result<String> {
        let payload = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "eth_call",
            "params": [{"to": to, "data": data}, "latest"]
        });
        let r = self.http.post(&self.rpc_url).json(&payload).send().await?.error_for_status()?;
        let env: RpcEnvelope<String> = r.json().await?;
        if let Some(err) = env.error {
            anyhow::bail!("Ethereum RPC eth_call error: {err}");
        }
        env.result.context("Ethereum RPC returned no result")
    }

    async fn eth_gas_price(&self) -> anyhow::Result<u128> {
        let payload = serde_json::json!({
            "jsonrpc": "2.0", "id": 1, "method": "eth_gasPrice", "params": []
        });
        let r = self.http.post(&self.rpc_url).json(&payload).send().await?.error_for_status()?;
        let env: RpcEnvelope<String> = r.json().await?;
        if let Some(err) = env.error { anyhow::bail!("Ethereum RPC eth_gasPrice error: {err}"); }
        let s = env.result.context("Ethereum RPC returned no gas price")?;
        Ok(u128::from_str_radix(s.trim_start_matches("0x"), 16)?)
    }
}

fn selector(signature: &str) -> Vec<u8> {
    let hash = Keccak256::digest(signature.as_bytes());
    hash[..4].to_vec()
}

fn decimal_to_raw(v: f64, decimals: u32) -> anyhow::Result<u128> {
    if !v.is_finite() || v < 0.0 { anyhow::bail!("invalid decimal amount"); }
    let scale = 10u128.pow(decimals);
    Ok((v * scale as f64).round() as u128)
}

fn raw_to_decimal(v: u128, decimals: u32) -> f64 {
    v as f64 / 10u128.pow(decimals) as f64
}

fn normalize_address(addr: &str) -> anyhow::Result<String> {
    let h = addr.trim_start_matches("0x");
    if h.len() != 40 || !h.chars().all(|c| c.is_ascii_hexdigit()) { anyhow::bail!("invalid EVM address"); }
    Ok(format!("0x{}", h.to_lowercase()))
}

fn word_address(addr: &str) -> anyhow::Result<Vec<u8>> {
    let h = normalize_address(addr)?;
    let raw = hex::decode(h.trim_start_matches("0x"))?;
    let mut out = vec![0u8; 12];
    out.extend(raw);
    Ok(out)
}

fn word_u128(v: u128) -> Vec<u8> {
    let mut out = vec![0u8; 16];
    out.extend(v.to_be_bytes());
    out
}

fn word_to_u128(word: &[u8]) -> anyhow::Result<u128> {
    if word.len() != 32 { anyhow::bail!("ABI word must be 32 bytes"); }
    if word[..16].iter().any(|b| *b != 0) { anyhow::bail!("ABI value does not fit u128"); }
    let mut a = [0u8; 16];
    a.copy_from_slice(&word[16..]);
    Ok(u128::from_be_bytes(a))
}

fn encode_approve(spender: &str, amount: u128) -> anyhow::Result<String> {
    let mut data = selector("approve(address,uint256)");
    data.extend(word_address(spender)?);
    data.extend(word_u128(amount));
    Ok(format!("0x{}", hex::encode(data)))
}

fn encode_allowance(owner: &str, spender: &str) -> anyhow::Result<String> {
    let mut data = selector("allowance(address,address)");
    data.extend(word_address(owner)?);
    data.extend(word_address(spender)?);
    Ok(format!("0x{}", hex::encode(data)))
}

fn encode_exact_input_single(
    token_in: &str,
    token_out: &str,
    fee: u32,
    recipient: &str,
    amount_in: u128,
    amount_out_min: u128,
) -> anyhow::Result<String> {
    // SwapRouter02 exactInputSingle((address,address,uint24,address,uint256,uint256,uint160))
    let mut data = selector("exactInputSingle((address,address,uint24,address,uint256,uint256,uint160))");
    data.extend(word_address(token_in)?);
    data.extend(word_address(token_out)?);
    data.extend(word_u128(fee as u128));
    data.extend(word_address(recipient)?);
    data.extend(word_u128(amount_in));
    data.extend(word_u128(amount_out_min));
    data.extend(word_u128(0));
    Ok(format!("0x{}", hex::encode(data)))
}
