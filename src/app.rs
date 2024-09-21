use clap::Parser;

#[derive(Parser, Debug)]
pub struct App {
    #[clap(flatten)]
    pub args: Args,

    #[clap(subcommand)]
    pub command: Command,
}

#[derive(Parser, Debug)]
#[command(version)]
pub struct Args {}

#[derive(Debug, Parser)]
pub enum Command {
    Launch {
        #[arg(long)]
        name: String,

        #[arg(long)]
        symbol: String,

        #[arg(long)]
        description: String,

        #[arg(long)]
        telegram: String,

        #[arg(long)]
        twitter: String,

        #[arg(long)]
        image_path: String,

        #[arg(long)]
        website: String,

        #[arg(long)]
        dev_buy: u64,

        #[arg(long)]
        snipe_buy: u64,
    },
    WalletsDrain {},
    Wallets {},
    LookForGeyser {},
    BundleStatus {
        #[arg(long)]
        bundle_id: String,
    },
    SubscribeTip {},
    GetTx {
        #[arg(long)]
        sig: String,
    },
    SlotCreated {
        #[arg(long)]
        mint: String,
    },
    SubscribePump {},
    TestSlotProgram {},
    SlotSubscribe {},
    IsOnCurve {
        #[arg(long)]
        pubkey: String,
    },
    Subscribe {},
    Seller {},
    BenchPump {},
    BenchPortal {},
    SnipePortal {
        #[arg(long)]
        lamports: u64,
    },
    SnipePump {
        #[arg(long)]
        lamports: u64,
    },
    Analyze {
        #[arg(long)]
        wallet_path: String,
    },
    Sanity {},
    CloseTokenAccounts {
        #[arg(long)]
        wallet_path: String,

        #[clap(long, default_value = "false")]
        burn: bool,
    },
    PumpService {
        #[arg(long)]
        lamports: u64,
    },
    BumpPump {
        #[arg(long)]
        mint: String,
    },
    SweepPump {
        #[arg(long)]
        wallet_path: String,
    },
    BuyPumpToken {
        #[arg(long)]
        mint: String,
    },
}
