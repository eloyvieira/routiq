let wallet = null;
const walletStatus = document.getElementById('walletStatus');
const result = document.getElementById('result');
const execution = document.getElementById('execution');
const health = document.getElementById('health');

async function apiJson(url, options = {}) {
  const r = await fetch(url, options);
  const text = await r.text();
  let body;
  try { body = JSON.parse(text); } catch { body = { error: text }; }
  if (!r.ok) throw new Error(body.error || text || `HTTP ${r.status}`);
  return body;
}

async function refreshHealth() {
  try { health.textContent = JSON.stringify(await apiJson('/api/health'), null, 2); }
  catch (e) { health.textContent = `Erro: ${e.message}`; }
}

document.getElementById('healthBtn').addEventListener('click', refreshHealth);
refreshHealth();

async function connectWallet() {
  if (!window.ethereum) throw new Error('MetaMask not found');
  const accounts = await window.ethereum.request({ method: 'eth_requestAccounts' });
  wallet = accounts[0] || null;
  walletStatus.textContent = wallet || 'Not connected';
  return wallet;
}

document.getElementById('connectWallet').addEventListener('click', async () => {
  try { await connectWallet(); } catch (e) { walletStatus.textContent = `erro: ${e.message}`; }
});

document.getElementById('quoteBtn').addEventListener('click', async () => {
  const amount = document.getElementById('amount').value;

  result.textContent = 'Querying Binance WebSocket + Uniswap QuoterV2...';

  try {
    const balances = await getWalletBalances();

    let url = `/api/quote?amount_usdt=${encodeURIComponent(amount)}`;

    if (balances) {
      url += `&wallet=${encodeURIComponent(balances.address)}`;
      url += `&wallet_usdt=${encodeURIComponent(balances.usdt_balance)}`;
      url += `&wallet_eth=${encodeURIComponent(balances.eth_balance)}`;
      url += `&wallet_weth=${encodeURIComponent(balances.weth_balance)}`;
    }

    const body = await apiJson(url);

    result.textContent = JSON.stringify(body, null, 2);
  } catch (e) {
    result.textContent = `Erro: ${e.message}`;
  }
});

async function getWalletBalances() {
  if (!window.ethereum) {
      return null;
  }

  const provider = new ethers.BrowserProvider(window.ethereum);

  const accounts = await provider.send("eth_accounts", []);

  if (!accounts.length) {
      return null;
  }

  const address = accounts[0];

  const ethBalanceRaw = await provider.getBalance(address);
  const ethBalance = Number(
      ethers.formatEther(ethBalanceRaw)
  );

  const USDT_ADDRESS =
      "0xdAC17F958D2ee523a2206206994597C13D831ec7";

  const WETH_ADDRESS =
      "0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2";

  const erc20Abi = [
      "function balanceOf(address owner) view returns (uint256)",
      "function decimals() view returns (uint8)"
  ];

  const usdt = new ethers.Contract(
      USDT_ADDRESS,
      erc20Abi,
      provider
  );

  const weth = new ethers.Contract(
      WETH_ADDRESS,
      erc20Abi,
      provider
  );

  const [
      usdtRaw,
      wethRaw
  ] = await Promise.all([
      usdt.balanceOf(address),
      weth.balanceOf(address)
  ]);

  return {
      address,
      eth_balance: ethBalance,
      usdt_balance: Number(
          ethers.formatUnits(usdtRaw, 6)
      ),
      weth_balance: Number(
        ethers.formatUnits(wethRaw, 18)
      )
  };
}

async function waitReceipt(txHash) {
  for (;;) {
    const receipt = await window.ethereum.request({
      method: 'eth_getTransactionReceipt',
      params: [txHash]
    });
    if (receipt) {
      if (receipt.status === '0x0') throw new Error(`Transaction reverted: ${txHash}`);
      return receipt;
    }
    await new Promise(r => setTimeout(r, 1500));
  }
}

async function ensureMainnet() {
  try {
    await window.ethereum.request({ method: 'wallet_switchEthereumChain', params: [{ chainId: '0x1' }] });
  } catch (e) {
    throw new Error(`Select Ethereum Mainnet in MetaMask: ${e.message}`);
  }
}

async function runDexPlan(plan) {
  if (!wallet) await connectWallet();
  await ensureMainnet();

  const allowanceHex = await window.ethereum.request({
    method: 'eth_call',
    params: [{ to: plan.token_in, data: plan.allowance_data }, 'latest']
  });
  const allowance = BigInt(allowanceHex || '0x0');
  const needed = BigInt(plan.amount_in_raw);

  if (allowance < needed) {
    if (allowance > 0n) {
      execution.textContent += '\nApproving USDT allowance reset to 0...';
      const resetHash = await window.ethereum.request({
        method: 'eth_sendTransaction',
        params: [{ from: wallet, to: plan.token_in, data: plan.approve_zero_data, value: '0x0' }]
      });
      await waitReceipt(resetHash);
    }

    execution.textContent += '\nApproving USDT for Uniswap SwapRouter02...';
    const approveHash = await window.ethereum.request({
      method: 'eth_sendTransaction',
      params: [{ from: wallet, to: plan.token_in, data: plan.approve_amount_data, value: '0x0' }]
    });
    await waitReceipt(approveHash);
  }

  execution.textContent += '\nSending USDT → WETH swap...';
  const swapHash = await window.ethereum.request({
    method: 'eth_sendTransaction',
    params: [{ from: wallet, to: plan.router, data: plan.swap_data, value: plan.value }]
  });
  await waitReceipt(swapHash);
  execution.textContent += `\nSwap confirmed: ${swapHash}`;
}

document.getElementById('executeBtn').addEventListener('click', async () => {
  const amount = Number(document.getElementById('amount').value);
  if (!Number.isFinite(amount) || amount <= 0) {
    execution.textContent = 'Invalid amount.';
    return;
  }

  if (!confirm(`Executar a melhor rota para trocar ${amount} USDT por ETH?`)) return;
  execution.textContent = 'Recalculating and preparing execution...';

  try {
    const body = await apiJson('/api/execute-best', {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ amount_usdt: amount, wallet, confirm: true })
    });
    execution.textContent = JSON.stringify(body, null, 2);

    if (body.action === 'dex_requires_wallet') {
      await runDexPlan(body.plan);
    }
  } catch (e) {
    execution.textContent += `\nErro: ${e.message}`;
  }
});
