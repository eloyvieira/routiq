# Routiq CEX/DEX Smart Router — USDT → ETH

Routiq is a cross-venue execution engine built in Rust that compares liquidity and execution costs between centralized and decentralized exchanges.

This version focuses on routing **USDT → ETH** between **Binance Spot** and **Uniswap V3 on Ethereum Mainnet**.

## Features

- Binance Spot market data via WebSocket
- Uniswap V3 quotes via QuoterV2
- CEX vs DEX execution comparison
- Effective output calculation
- Binance market order execution
- MetaMask-based Uniswap execution
- Rust backend with Axum and Tokio
- Web interface served by the Rust application

## Installation

Install Rust:

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source "$HOME/.cargo/env"
```

Create the environment file:

```bash
cp .env.example .env
```

Load environment variables and start the application:

```bash
set -a
source .env
set +a

cargo run
```

Open the web interface:

```text
http://127.0.0.1:8082
```

You can also access it using the server IP:

```text
http://SERVER_IP:8082
```

## Ethereum Configuration

Set an Ethereum Mainnet RPC endpoint:

```env
ETH_RPC_URL=https://YOUR_ETHEREUM_RPC
```

A public RPC can be used for development, but a dedicated RPC provider is recommended for production environments.

Example:

```env
ETH_RPC_URL=https://ethereum-rpc.publicnode.com
```

## Binance Configuration

Public Binance market data does not require API credentials.

Keep live trading disabled while testing:

```env
ENABLE_LIVE_TRADING=false
```

To enable Binance Spot execution, configure an API key with Spot Trading enabled and withdrawals disabled:

```env
BINANCE_API_KEY=...
BINANCE_API_SECRET=...
ENABLE_LIVE_TRADING=true
```

The current Binance execution uses:

```text
Symbol: ETHUSDT
Side: BUY
Type: MARKET
quoteOrderQty: <USDT amount>
```

Example:

```text
100 USDT
   ↓
ETHUSDT MARKET BUY
   ↓
ETH

## Uniswap Configuration

Routiq currently uses Uniswap V3 on Ethereum Mainnet.

Contracts:

- USDT: `0xdAC17F958D2ee523a2206206994597C13D831ec7`
- WETH: `0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2`
- QuoterV2: `0x61fFE014bA17989E743c5F6cB21bF9697530B21e`
- SwapRouter02: `0x68b3465833fb72A70ecDF485E0e4C7bD8665Fc45`

DEX execution flow:

1. Connect MetaMask
2. Switch to Ethereum Mainnet
3. Check the USDT allowance
4. Approve USDT when required
5. Send `exactInputSingle` to SwapRouter02
6. Wait for the transaction receipt

The current DEX output is **WETH**, the ERC-20 representation of ETH used by the router.

Future versions can add automatic WETH unwrap or use the Universal Router / multicall flow to return native ETH.

## API Endpoints

### Health Check

```http
GET /api/health
```

### Compare Routes

Example with 100 USDT:

```http
GET /api/quote?amount_usdt=100
```

The API compares the effective ETH output from Binance and Uniswap.

### Execute Best Route

```http
POST /api/execute-best
Content-Type: application/json
```

Request:

```json
{
  "amount_usdt": 100,
  "wallet": "0xYourWallet",
  "confirm": true
}
```

Before execution, Routiq recalculates the route.

If Binance provides the best execution and live trading is enabled, the backend submits the Spot market order.

If Uniswap provides the best execution, Routiq returns a transaction plan that is signed and confirmed through MetaMask.

## Architecture

```text
Market Data Layer
        ↓
Execution Layer
        ↓
Routing Layer
        ↓
Risk Layer
        ↓
Portfolio Layer
        ↓
Strategy Layer
```

## Current Limitations

- Binance uses top-20 depth only
- Binance fees are configured manually
- No CEX/DEX split routing yet
- Uniswap supports Ethereum Mainnet only
- Current DEX flow is USDT → WETH
- Capital must already exist on both venues
- Production safeguards are not implemented yet

## Roadmap

- Full Binance order book
- Dynamic fee detection
- Multi-venue routing
- Split execution
- More CEX and DEX integrations
- WETH → ETH unwrap
- Portfolio and PnL tracking
- Order persistence
- Risk controls