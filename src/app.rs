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
    PumpService {},
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
