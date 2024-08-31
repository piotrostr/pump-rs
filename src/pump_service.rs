use crate::jito::{
    start_bundle_results_listener, subscribe_tips, SearcherClient,
};
use crate::pump::{self, PumpBuyRequest};
use crate::slot::make_deadline_tx;
use crate::util::{get_jito_tip_pubkey, make_compute_budget_ixs};
use actix_web::web::Data;
use actix_web::{get, post, web::Json, App, Error, HttpResponse, HttpServer};

use jito_searcher_client::{get_searcher_client, send_bundle_no_wait};
use log::{debug, error, info, warn};
use serde::{Deserialize, Serialize};
use serde_json::json;
use solana_client::nonblocking::rpc_client::RpcClient;
use solana_sdk::clock::Slot;
use solana_sdk::hash::Hash;
use solana_sdk::signature::Keypair;
use solana_sdk::signer::{EncodableKey, Signer};
use solana_sdk::system_instruction::transfer;

use solana_sdk::transaction::{Transaction, VersionedTransaction};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{Mutex, RwLock};
use tokio::time::interval;

fn env(key: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| panic!("{} env var not set", key))
}

pub async fn update_latest_blockhash(
    rpc_client: Arc<RpcClient>,
    latest_blockhash: Arc<RwLock<Hash>>,
) {
    let mut interval = interval(Duration::from_secs(1));
    loop {
        interval.tick().await;
        match rpc_client.get_latest_blockhash().await {
            Ok(new_blockhash) => {
                let mut blockhash = latest_blockhash.write().await;
                *blockhash = new_blockhash;
                debug!("Updated latest blockhash: {}", new_blockhash);
            }
            Err(e) => {
                error!("Failed to get latest blockhash: {}", e);
            }
        }
    }
}

pub struct AppState {
    pub wallet: Arc<Mutex<Keypair>>,
    pub searcher_client: Arc<Mutex<SearcherClient>>,
    pub latest_blockhash: Arc<RwLock<Hash>>,
    pub dynamic_tip: Arc<RwLock<u64>>,
    pub lamports: u64,
}

#[get("/blockhash")]
#[timed::timed(duration(printer = "info!"))]
pub async fn get_blockhash(state: Data<AppState>) -> HttpResponse {
    let blockhash = state.latest_blockhash.read().await;
    HttpResponse::Ok().json(json!({
        "blockhash": blockhash.to_string()
    }))
}

use crate::util::{pubkey_to_string, string_to_pubkey};
use solana_sdk::pubkey::Pubkey;
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreatePumpTokenEvent {
    pub sig: String,
    pub slot: Slot,
    #[serde(
        serialize_with = "pubkey_to_string",
        deserialize_with = "string_to_pubkey"
    )]
    pub mint: Pubkey,
    #[serde(
        serialize_with = "pubkey_to_string",
        deserialize_with = "string_to_pubkey"
    )]
    pub bounding_curve: Pubkey,
    #[serde(
        serialize_with = "pubkey_to_string",
        deserialize_with = "string_to_pubkey"
    )]
    pub associated_bounding_curve: Pubkey,
    pub name: String,
    pub symbol: String,
    pub uri: String,
    pub dev_bought_amount: u64,
    pub dev_max_sol_cost: u64,
    pub num_dev_buy_txs: u64,
    pub virtual_sol_reserves: u64,
    pub virtual_token_reserves: u64,
}

#[post("/v2/pump-buy")]
#[timed::timed(duration(printer = "info!"))]
pub async fn handle_pump_buy_v2(
    create_pump_token_event: Json<CreatePumpTokenEvent>,
    state: Data<AppState>,
) -> Result<HttpResponse, Error> {
    info!("handling pump event {}", create_pump_token_event.sig);
    let mint = create_pump_token_event.mint;
    let pump_buy_request = PumpBuyRequest {
        mint: create_pump_token_event.mint,
        bonding_curve: create_pump_token_event.bounding_curve,
        associated_bonding_curve: create_pump_token_event
            .associated_bounding_curve,
        virtual_sol_reserves: create_pump_token_event.virtual_sol_reserves,
        virtual_token_reserves: create_pump_token_event
            .virtual_token_reserves,
        slot: Some(create_pump_token_event.slot),
    };
    if create_pump_token_event.dev_max_sol_cost > 1_500_000_000 {
        warn!("dev_max_sol_cost too high");
        return Ok(HttpResponse::Ok().json(json!({
            "status": "OK, but dev bought amount too high"
        })));
    }
    let wallet = state.wallet.lock().await;
    let mut searcher_client = state.searcher_client.lock().await;
    let latest_blockhash = state.latest_blockhash.read().await;
    let dynamic_tip = state.dynamic_tip.read().await;
    let deadline = create_pump_token_event.slot + 1;
    _handle_pump_buy(
        pump_buy_request,
        state.lamports,
        *dynamic_tip,
        &wallet,
        &mut searcher_client,
        &latest_blockhash,
        Some(deadline),
        5,
        3,
    )
    .await?;

    Ok(HttpResponse::Ok().json(json!({
    "status": format!(
        "OK, trigerred buy of {}", mint.to_string())
    })))
}

#[post("/pump-buy")]
#[timed::timed(duration(printer = "info!"))]
pub async fn handle_pump_buy(
    pump_buy_request: Json<PumpBuyRequest>,
    state: Data<AppState>,
) -> Result<HttpResponse, Error> {
    info!(
        "handling pump buy req {}",
        serde_json::to_string_pretty(&pump_buy_request)?
    );
    let mint = pump_buy_request.mint;
    let pump_buy_request = pump_buy_request.clone();
    let wallet = state.wallet.lock().await;
    let mut searcher_client = state.searcher_client.lock().await;
    let latest_blockhash = state.latest_blockhash.read().await;
    let dynamic_tip = state.dynamic_tip.read().await;
    let deadline = if pump_buy_request.slot.is_some() {
        Some(pump_buy_request.slot.unwrap() + 1)
    } else {
        None
    };
    _handle_pump_buy(
        pump_buy_request,
        state.lamports,
        *dynamic_tip,
        &wallet,
        &mut searcher_client,
        &latest_blockhash,
        deadline,
        69,
        3,
    )
    .await?;
    Ok(HttpResponse::Ok().json(json!({
    "status": format!(
        "OK, trigerred buy of {}",
        mint.to_string())
    })))
}

/// apply_fee puts the 1% pump fee on the lamports in
/// e.g. 1 sol -> 1.01 sol
pub fn apply_fee(lamports: u64) -> u64 {
    let fee = lamports / 100;
    lamports + fee
}

#[timed::timed(duration(printer = "info!"))]
pub async fn _handle_pump_buy(
    pump_buy_request: PumpBuyRequest,
    lamports: u64,
    tip: u64,
    wallet: &Keypair,
    searcher_client: &mut SearcherClient,
    latest_blockhash: &Hash,
    deadline: Option<u64>,
    jitter: u64,
    num_tries: usize,
) -> Result<(), Box<dyn std::error::Error>> {
    // Calculate token amount once, using the original lamports value
    let token_amount = pump::get_token_amount(
        pump_buy_request.virtual_sol_reserves,
        pump_buy_request.virtual_token_reserves,
        None,
        lamports,
    )?;

    let mut jitter = jitter;
    for i in 0..num_tries {
        let mut ixs = vec![];
        ixs.append(&mut make_compute_budget_ixs(1000069, 72014));
        ixs.append(&mut pump::_make_buy_ixs(
            wallet.pubkey(),
            pump_buy_request.mint,
            pump_buy_request.bonding_curve,
            pump_buy_request.associated_bonding_curve,
            token_amount,
            // add random lamports in order to arrive at different sigs
            // (jito pubkey itself probably works too)
            apply_fee(lamports) + jitter + i as u64,
        )?);
        ixs.push(transfer(&wallet.pubkey(), &get_jito_tip_pubkey(), tip));

        let swap_tx = Transaction::new_signed_with_payer(
            ixs.as_slice(),
            Some(&wallet.pubkey()),
            &[wallet],
            *latest_blockhash,
        );

        let txs = if let Some(deadline) = deadline {
            vec![
                make_deadline_tx(deadline, *latest_blockhash, wallet),
                swap_tx,
            ]
        } else {
            vec![swap_tx]
        };

        let versioned_txs: Vec<VersionedTransaction> = txs
            .iter()
            .map(|tx| VersionedTransaction::from(tx.clone()))
            .collect();

        tokio::time::sleep(Duration::from_millis(jitter)).await;
        let res = send_bundle_no_wait(&versioned_txs, searcher_client)
            .await
            .expect("send bundle no wait");

        info!(
            "Bundle {} sent through gRPC: {} {:#?}",
            i + 1,
            res.into_inner().uuid,
            versioned_txs
                .iter()
                .map(|tx| tx.signatures[0])
                .collect::<Vec<_>>()
        );

        jitter += 1;
    }

    // let start = std::time::Instant::now();
    // for (i, bundle) in bundles.iter().enumerate() {
    //     let txs: Vec<Transaction> = bundle
    //         .iter()
    //         .map(|vtx| {
    //             Transaction::try_from(vtx.clone())
    //                 .expect("Failed to convert to Transaction")
    //         })
    //         .collect();
    //     send_out_bundle_to_all_regions(&txs).await?;

    //     info!("Bundle {} sent out through HTTP", i + 1);

    //     // Add jitter between 0 and 200ms
    //     let jitter = rng.gen_range(0..200);
    //     tokio::time::sleep(Duration::from_millis(jitter)).await;
    // }

    // let elapsed = start.elapsed();
    // info!("All bundles sent out through HTTP in {:?}", elapsed);

    Ok(())
}

#[get("/healthz")]
#[timed::timed(duration(printer = "info!"))]
pub async fn healthz(request: actix_web::HttpRequest) -> HttpResponse {
    info!(
        "healthz from {}",
        request.connection_info().peer_addr().unwrap_or("unknown")
    );
    HttpResponse::Ok().json(json!({
        "status": "im ok, hit me with pump stuff"
    }))
}

pub async fn run_pump_service(lamports: u64) -> std::io::Result<()> {
    // keep all of the state in the app state not to re-init
    let wallet = Arc::new(Mutex::new(
        Keypair::read_from_file(env("FUND_KEYPAIR_PATH"))
            .expect("read fund keypair"),
    ));
    let auth =
        Arc::new(Keypair::read_from_file(env("AUTH_KEYPAIR_PATH")).unwrap());
    let searcher_client = Arc::new(Mutex::new(
        get_searcher_client(env("BLOCK_ENGINE_URL").as_str(), &auth)
            .await
            .expect("makes searcher client"),
    ));

    start_bundle_results_listener(searcher_client.clone()).await;

    let dynamic_tip = Arc::new(RwLock::new(0));
    subscribe_tips(dynamic_tip.clone());

    let app_state = Data::new(AppState {
        wallet,
        searcher_client,
        latest_blockhash: Arc::new(RwLock::new(Hash::default())),
        dynamic_tip,
        lamports,
    });

    // poll for latest blockhash to trim 200ms
    let rpc_client = Arc::new(RpcClient::new(env("RPC_URL")));
    tokio::spawn(update_latest_blockhash(
        rpc_client.clone(),
        app_state.latest_blockhash.clone(),
    ));

    info!("Running pump service on 6969");
    HttpServer::new(move || {
        App::new()
            .service(handle_pump_buy)
            .service(handle_pump_buy_v2)
            .service(get_blockhash)
            .service(healthz)
            .app_data(app_state.clone())
    })
    .bind(("0.0.0.0", 6969))?
    .run()
    .await
}
