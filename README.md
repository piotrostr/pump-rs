# pump-rs

bunch of pump.fun related utilities, centered around sniping new launches

i think this is a flex, check [final run](https://github.com/piotrostr/pump-rs/blob/master/runs/final.txt)

## Journey

this was a rabbit hole

went from listening on pump.fun stream from the frontend API (avg ~5-10 slots),
to using pumpportal API for data (avg 1-5 slots) to using a custom on-chain
program with deadline control and deserializing raw shreds from Jito
Shredstream (avg 0.5 slots over 100+ entries)

putting this on halt and open-sourcing since the market is dead (as of 1st Sep)

the shredstream webhook (`/v2/pump-buy/`) is currently private, I might
open-source too but needs some more polishing before that:)

## Features

the profits of this bot stem from being faster than other snipers and dumping
on them, have only caught one raydium seed (15x) in late July

there are some sniper bots (once profitable) that hold a token and sell later
if it turns out to be a good one, but right now it is simply not worth it,
since 99% of launches fail to reach the bonding curve (if not 99.9%)

strategy for `seller-service` for this one is to wait for the confirmation of
tx to arrive through the PubSub client and sell straight-away

```txt
Usage: pump-rs <COMMAND>

Commands:
  wallets
  look-for-geyser
  bundle-status
  subscribe-tip
  get-tx
  slot-created
  subscribe-pump
  test-slot-program
  slot-subscribe
  is-on-curve
  subscribe
  seller
  bench-pump
  bench-portal
  snipe-portal
  snipe-pump
  analyze
  sanity
  close-token-accounts
  pump-service
  bump-pump
  sweep-pump
  buy-pump-token
  help                  Print this message or the help of the given subcommand(s)

Options:
  -h, --help     Print help
  -V, --version  Print version
```

Disclaimer: this is provided as-is, it is unlikely to be profitable for you,
this runs on mainnet and it can cause losses

## License

MIT
