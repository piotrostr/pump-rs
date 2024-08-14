use crate::constants::SLOT_CHECKER_MAINNET;
use crate::pump::{self, PumpBuyRequest, SearcherClient};
use crate::util::{get_jito_tip_pubkey, make_compute_budget_ixs};
use actix_web::web::Data;
use actix_web::{get, post, web::Json, App, Error, HttpResponse, HttpServer};
use futures_util::StreamExt;
use jito_protos::bundle::{bundle_result, BundleResult};
use jito_protos::searcher::SubscribeBundleResultsRequest;

use jito_searcher_client::{get_searcher_client, send_bundle_no_wait};
use log::{debug, error, info};
use serde_json::json;
use solana_client::nonblocking::pubsub_client::PubsubClient;
use solana_client::nonblocking::rpc_client::RpcClient;
use solana_sdk::hash::Hash;
use solana_sdk::instruction::Instruction;
use solana_sdk::pubkey::Pubkey;
use solana_sdk::signature::Keypair;
use solana_sdk::signer::{EncodableKey, Signer};
use solana_sdk::system_instruction::transfer;

use solana_sdk::transaction::{Transaction, VersionedTransaction};
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{Mutex, RwLock};
use tokio::task::JoinHandle;
use tokio::time::interval;

fn env(key: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| panic!("{} env var not set", key))
}

pub fn make_deadline_tx(
    deadline: u64,
    latest_blockhash: Hash,
    keypair: &Keypair,
) -> VersionedTransaction {
    VersionedTransaction::from(Transaction::new_signed_with_payer(
        &[Instruction::new_with_bytes(
            Pubkey::from_str(SLOT_CHECKER_MAINNET).expect("pubkey"),
            &deadline.to_le_bytes(),
            vec![],
        )],
        Some(&keypair.pubkey()),
        &[keypair],
        latest_blockhash,
    ))
}

pub fn update_slot(current_slot: Arc<RwLock<u64>>) -> JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            let pubsub_client = PubsubClient::new(&env("WS_URL"))
                .await
                .expect("pubsub client");
            let (mut stream, unsub) = pubsub_client
                .slot_subscribe()
                .await
                .expect("slot subscribe");
            while let Some(slot_info) = stream.next().await {
                let mut current_slot = current_slot.write().await;
                *current_slot = slot_info.slot;
                debug!("Updated slot: {}", current_slot);
            }
            unsub().await;
            // wait for a second before reconnecting
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    })
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
}

#[get("/blockhash")]
#[timed::timed(duration(printer = "info!"))]
pub async fn get_blockhash(state: Data<AppState>) -> HttpResponse {
    let blockhash = state.latest_blockhash.read().await;
    HttpResponse::Ok().json(json!({
        "blockhash": blockhash.to_string()
    }))
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
    let lamports = 50_000_000;
    let tip = 200000;

    let mint = pump_buy_request.mint;
    let pump_buy_request = pump_buy_request.clone();
    let wallet = state.wallet.lock().await;
    let mut searcher_client = state.searcher_client.lock().await;
    let latest_blockhash = state.latest_blockhash.read().await;
    _handle_pump_buy(
        pump_buy_request,
        lamports,
        tip,
        &wallet,
        &mut searcher_client,
        &latest_blockhash,
        None,
    )
    .await?;
    Ok(HttpResponse::Ok().json(json!({
    "status": format!(
        "OK, trigerred buy of {}",
        mint.to_string())
    })))
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
) -> Result<(), Box<dyn std::error::Error>> {
    let token_amount = pump::get_token_amount(
        pump_buy_request.virtual_sol_reserves,
        pump_buy_request.virtual_token_reserves,
        None,
        lamports,
    )?;
    let token_amount = (token_amount as f64 * 0.95) as u64;
    let mut ixs = vec![];
    ixs.append(&mut make_compute_budget_ixs(1000069, 72014));
    ixs.append(&mut pump::_make_buy_ixs(
        wallet.pubkey(),
        pump_buy_request.mint,
        pump_buy_request.bonding_curve,
        pump_buy_request.associated_bonding_curve,
        token_amount,
        lamports,
    )?);
    ixs.push(transfer(&wallet.pubkey(), &get_jito_tip_pubkey(), tip));
    let swap_tx =
        VersionedTransaction::from(Transaction::new_signed_with_payer(
            ixs.as_slice(),
            Some(&wallet.pubkey()),
            &[wallet],
            *latest_blockhash,
        ));
    let start = std::time::Instant::now();
    let txs = if let Some(deadline) = deadline {
        vec![
            make_deadline_tx(deadline, *latest_blockhash, wallet),
            swap_tx,
        ]
    } else {
        vec![swap_tx]
    };
    let res = send_bundle_no_wait(&txs, searcher_client)
        .await
        .expect("send bundle no wait");
    let elapsed = start.elapsed();
    info!("Bundle sent in {:?}", elapsed);
    info!("Bundle sent. UUID: {}", res.into_inner().uuid);
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

pub async fn run_pump_service() -> std::io::Result<()> {
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

    let app_state = Data::new(AppState {
        wallet,
        searcher_client,
        latest_blockhash: Arc::new(RwLock::new(Hash::default())),
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
            .service(get_blockhash)
            .service(healthz)
            .app_data(app_state.clone())
    })
    .bind(("0.0.0.0", 6969))?
    .run()
    .await
}

#[timed::timed(duration(printer = "info!"))]
pub async fn start_bundle_results_listener(
    searcher_client: Arc<Mutex<SearcherClient>>,
) {
    // keep a stream for bundle results
    // TODO hopefully this doesn't deadlock
    let mut bundle_results_stream = searcher_client
        .lock()
        .await
        .subscribe_bundle_results(SubscribeBundleResultsRequest {})
        .await
        .expect("subscribe bundle results")
        .into_inner();
    // poll for bundle results
    tokio::spawn(async move {
        while let Some(res) = bundle_results_stream.next().await {
            if let Ok(BundleResult {
                bundle_id,
                result: Some(result),
            }) = res
            {
                match result {
                    bundle_result::Result::Accepted(_) => {
                        info!("Bundle {} accepted", bundle_id);
                    }
                    bundle_result::Result::Rejected(rejection) => {
                        info!(
                            "Bundle {} rejected: {:?}",
                            bundle_id, rejection
                        );
                    }
                    bundle_result::Result::Dropped(_) => {
                        info!("Bundle {} dropped", bundle_id);
                    }
                    bundle_result::Result::Processed(_) => {
                        info!("Bundle {} processed", bundle_id);
                    }
                    bundle_result::Result::Finalized(_) => {
                        info!("Bundle {} finalized", bundle_id);
                    }
                }
            }
        }
    });
}
