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
    Sanity {},
    CloseTokenAccounts {
        #[arg(long)]
        wallet_path: String,
    },
    PumpService {},
    SellPump {
        #[arg(long)]
        mint: String,
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
