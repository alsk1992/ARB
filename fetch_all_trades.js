// Fetch all trades for a wallet with proper pagination
import fs from 'fs';

const WALLET = process.argv[2] || '0xbefbdd434fc8d99da3e37c20cb0f088ec3168a78';
const OUTPUT = process.argv[3] || 'all_trades.json';

async function fetchAllTrades(wallet) {
  let allTrades = [];
  let offset = 0;
  const limit = 500;
  let hasMore = true;

  console.log(`Fetching trades for ${wallet}...`);

  while (hasMore) {
    const url = `https://data-api.polymarket.com/activity?user=${wallet}&limit=${limit}&offset=${offset}`;
    console.log(`Fetching offset ${offset}...`);

    const res = await fetch(url);
    const data = await res.json();

    if (!data || data.length === 0) {
      hasMore = false;
      break;
    }

    // Filter only TRADE type
    const trades = data.filter(d => d.type === 'TRADE');
    allTrades = allTrades.concat(trades);

    console.log(`  Got ${trades.length} trades (total: ${allTrades.length})`);

    // Check if we got less than limit (means we're at the end)
    if (data.length < limit) {
      hasMore = false;
    }

    offset += limit;

    // Safety limit
    if (offset > 100000) {
      console.log('Hit safety limit');
      break;
    }

    // Small delay to avoid rate limiting
    await new Promise(r => setTimeout(r, 100));
  }

  // Sort by timestamp ascending
  allTrades.sort((a, b) => a.timestamp - b.timestamp);

  return allTrades;
}

async function main() {
  const trades = await fetchAllTrades(WALLET);

  console.log(`\nTotal trades fetched: ${trades.length}`);

  // Get unique markets
  const markets = [...new Set(trades.map(t => t.title))];
  console.log(`Unique markets: ${markets.length}`);

  // Save to file
  fs.writeFileSync(OUTPUT, JSON.stringify(trades, null, 2));
  console.log(`Saved to ${OUTPUT}`);

  // Quick stats
  const byMarket = {};
  trades.forEach(t => {
    if (!byMarket[t.title]) {
      byMarket[t.title] = { up: [], down: [] };
    }
    byMarket[t.title][t.outcome.toLowerCase()].push(t);
  });

  console.log('\n--- Per Market Summary ---');
  Object.keys(byMarket).slice(0, 10).forEach(market => {
    const m = byMarket[market];
    const upCost = m.up.reduce((s, t) => s + t.usdcSize, 0);
    const downCost = m.down.reduce((s, t) => s + t.usdcSize, 0);
    const upShares = m.up.reduce((s, t) => s + t.size, 0);
    const downShares = m.down.reduce((s, t) => s + t.size, 0);
    console.log(`\n${market}`);
    console.log(`  UP: ${m.up.length} trades, $${upCost.toFixed(2)} cost, ${upShares.toFixed(2)} shares`);
    console.log(`  DOWN: ${m.down.length} trades, $${downCost.toFixed(2)} cost, ${downShares.toFixed(2)} shares`);
  });
}

main().catch(console.error);
