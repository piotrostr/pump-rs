use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use borsh::{BorshDeserialize, BorshSerialize};
use log::debug;
use rand::Rng;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use solana_client::nonblocking::rpc_client::RpcClient;
use solana_sdk::{
    instruction::{AccountMeta, Instruction},
    pubkey::Pubkey,
    signature::Keypair,
    signer::Signer,
    transaction::Transaction,
};
use std::str::FromStr;

use crate::util::make_compute_budget_ixs;
use crate::{
    constants::{
        ASSOCIATED_TOKEN_PROGRAM, EVENT_AUTHORITY, PUMP_FUN_MINT_AUTHORITY,
        PUMP_FUN_PROGRAM, PUMP_GLOBAL_ADDRESS, RENT_PROGRAM,
        SYSTEM_PROGRAM_ID, TOKEN_PROGRAM,
    },
    util::env,
};

pub const MPL_TOKEN_METADATA: &str =
    "metaqbxxUerdq28cj1RbAWkYQm3ybzjb6a8bt518x1s";
// this might be derived
pub const METADATA: &str = "GgrH3ApmK1SYJVZNEuUavbZQx4Yt8WoBz3tkRuLKwj45";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IPFSMetaForm {
    pub name: String,
    pub symbol: String,
    pub description: String,
    pub twitter: String,
    pub telegram: String,
    pub website: String,
    #[serde(rename = "showName")]
    pub show_name: bool,
}

impl IPFSMetaForm {
    pub fn new(name: String, symbol: String, description: String) -> Self {
        Self {
            name,
            symbol,
            description,
            show_name: true,
            telegram: String::new(),
            twitter: String::new(),
            website: String::new(),
        }
    }
}

fn generate_random_image() -> Vec<u8> {
    let mut rng = rand::thread_rng();
    let width = 100;
    let height = 100;
    let mut image_data = Vec::with_capacity(width * height * 3);

    for _ in 0..(width * height) {
        image_data.push(rng.gen());
        image_data.push(rng.gen());
        image_data.push(rng.gen());
    }

    image_data
}

#[derive(BorshSerialize, BorshDeserialize, Debug)]
pub struct PumpCreateTokenIx {
    pub method_id: [u8; 8],
    pub name: String,
    pub symbol: String,
    pub uri: String,
}

impl PumpCreateTokenIx {
    pub fn new(name: String, symbol: String, uri: String) -> Self {
        Self {
            method_id: [0x18, 0x1e, 0xc8, 0x28, 0x05, 0x1c, 0x07, 0x77],
            name,
            symbol,
            uri,
        }
    }
}

pub async fn push_image_to_ipfs(
    client: &Client,
    image: Vec<u8>,
) -> Result<String, Box<dyn std::error::Error>> {
    let form = reqwest::multipart::Form::new()
        .part("file", reqwest::multipart::Part::bytes(image));

    let res = client
        .post("https://ipfs.infura.io:5001/api/v0/add")
        .multipart(form)
        .send()
        .await?
        .json::<serde_json::Value>()
        .await?;

    Ok(res["Hash"].as_str().unwrap().to_string())
}

pub async fn push_meta_onto_ipfs(
    client: &Client,
    ipfs_meta: &IPFSMetaForm,
) -> Result<String, Box<dyn std::error::Error>> {
    let data = serde_json::to_vec(ipfs_meta)?;
    let form = reqwest::multipart::Form::new()
        .part("file", reqwest::multipart::Part::bytes(data));

    let res = client
        .post("https://ipfs.infura.io:5001/api/v0/add")
        .multipart(form)
        .send()
        .await?
        .json::<serde_json::Value>()
        .await?;

    Ok("https://ipfs.io/ipfs/".to_string() + res["Hash"].as_str().unwrap())
}

pub async fn push_meta_to_pump_ipfs(
    client: &Client,
    ipfs_meta: &IPFSMetaForm,
    image: Vec<u8>,
) -> Result<String, Box<dyn std::error::Error>> {
    let form = reqwest::multipart::Form::new()
        .text("name", ipfs_meta.name.clone())
        .text("symbol", ipfs_meta.symbol.clone())
        .text("description", ipfs_meta.description.clone())
        .text("twitter", ipfs_meta.twitter.clone())
        .text("telegram", ipfs_meta.telegram.clone())
        .text("website", ipfs_meta.website.clone())
        .text("showName", ipfs_meta.show_name.to_string())
        .part(
            "file",
            reqwest::multipart::Part::bytes(image)
                .file_name("image.png")
                .mime_str("image/png")?,
        );

    let res = client
        .post("https://pump.fun/api/ipfs")
        .multipart(form)
        .send()
        .await?
        .json::<serde_json::Value>()
        .await?;

    println!("res: {:#?}", res);

    Ok(res["metadataUri"].as_str().unwrap().to_string())
}

pub fn generate_mint() -> (Pubkey, Keypair) {
    let keypair = Keypair::new();
    let pubkey = keypair.pubkey();
    (pubkey, keypair)
}

pub async fn launch(
    name: String,
    symbol: String,
    description: String,
    signer: &Keypair,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut ixs = vec![];

    // Add compute budget instructions
    ixs.append(&mut make_compute_budget_ixs(542850, 250000));

    let image = generate_random_image();
    // Generate and push random image to IPFS
    let client = get_ipfs_client();
    let ipfs_meta =
        IPFSMetaForm::new(name.clone(), symbol.clone(), description);
    let metadata_uri =
        push_meta_to_pump_ipfs(&client, &ipfs_meta, image).await?;
    let (mint, mint_signer) = generate_mint();

    ixs.push(_make_create_token_ix(
        name,
        symbol,
        metadata_uri,
        mint,
        signer.pubkey(),
    ));

    let rpc_client = RpcClient::new(env("RPC_URL"));
    rpc_client
        .send_and_confirm_transaction_with_spinner(
            &Transaction::new_signed_with_payer(
                &ixs,
                Some(&signer.pubkey()),
                &[signer, &mint_signer],
                rpc_client.get_latest_blockhash().await?,
            ),
        )
        .await?;

    Ok(())
}

pub fn get_bc_and_abc(mint: Pubkey) -> (Pubkey, Pubkey) {
    let (bonding_curve, _) = Pubkey::find_program_address(
        &[b"bonding-curve", mint.as_ref()],
        &Pubkey::from_str(PUMP_FUN_PROGRAM).unwrap(),
    );

    // Derive the associated bonding curve address
    let associated_bonding_curve =
        spl_associated_token_account::get_associated_token_address(
            &bonding_curve,
            &mint,
        );

    (bonding_curve, associated_bonding_curve)
}

pub fn _make_create_token_ix(
    name: String,
    symbol: String,
    metadata_uri: String,
    mint: Pubkey,
    user: Pubkey,
) -> Instruction {
    // Construct the instruction data
    let instruction_data = PumpCreateTokenIx::new(name, symbol, metadata_uri);

    let metadata = derive_metadata_account(&mint);
    let (bonding_curve, associated_bonding_curve) = get_bc_and_abc(mint);

    debug!("instruction_data: {:#?}", instruction_data);
    // serialize borsh to hex string
    let mut buffer = Vec::new();
    instruction_data.serialize(&mut buffer).unwrap();
    debug!("hex: {}", hex::encode(buffer));

    // Create the main instruction
    let accounts = vec![
        AccountMeta::new(mint, true),
        AccountMeta::new_readonly(
            Pubkey::from_str(PUMP_FUN_MINT_AUTHORITY).unwrap(),
            false,
        ),
        AccountMeta::new(bonding_curve, false),
        AccountMeta::new(associated_bonding_curve, false),
        AccountMeta::new_readonly(
            Pubkey::from_str(PUMP_GLOBAL_ADDRESS).unwrap(),
            false,
        ),
        AccountMeta::new_readonly(
            Pubkey::from_str(MPL_TOKEN_METADATA).unwrap(),
            false,
        ),
        AccountMeta::new(metadata, false),
        AccountMeta::new(user, true),
        AccountMeta::new_readonly(
            Pubkey::from_str(SYSTEM_PROGRAM_ID).unwrap(),
            false,
        ),
        AccountMeta::new_readonly(
            Pubkey::from_str(TOKEN_PROGRAM).unwrap(),
            false,
        ),
        AccountMeta::new_readonly(
            Pubkey::from_str(ASSOCIATED_TOKEN_PROGRAM).unwrap(),
            false,
        ),
        AccountMeta::new_readonly(
            Pubkey::from_str(RENT_PROGRAM).unwrap(),
            false,
        ),
        AccountMeta::new_readonly(
            Pubkey::from_str(EVENT_AUTHORITY).unwrap(),
            false,
        ),
        AccountMeta::new_readonly(
            Pubkey::from_str(PUMP_FUN_PROGRAM).unwrap(),
            false,
        ),
    ];

    debug!("accounts: {:#?}", accounts);

    Instruction::new_with_borsh(
        Pubkey::from_str(PUMP_FUN_PROGRAM).unwrap(),
        &instruction_data,
        accounts,
    )
}

fn get_ipfs_client() -> Client {
    dotenv::dotenv().ok();
    let project_id =
        std::env::var("INFURA_PROJECT").expect("INFURA_PROJECT must be set");
    let project_secret =
        std::env::var("INFURA_SECRET").expect("INFURA_SECRET must be set");

    let auth = format!("{}:{}", project_id, project_secret);
    let encoded_auth = BASE64.encode(auth);

    Client::builder()
        .default_headers({
            let mut headers = reqwest::header::HeaderMap::new();
            headers.insert(
                reqwest::header::AUTHORIZATION,
                format!("Basic {}", encoded_auth).parse().unwrap(),
            );
            headers
        })
        .build()
        .expect("Failed to create IPFS client")
}

pub fn derive_metadata_account(mint: &Pubkey) -> Pubkey {
    let metaplex_program_id = Pubkey::from_str(MPL_TOKEN_METADATA).unwrap();

    Pubkey::find_program_address(
        &[b"metadata", metaplex_program_id.as_ref(), mint.as_ref()],
        &metaplex_program_id,
    )
    .0
}

#[cfg(test)]
mod tests {
    use solana_sdk::signer::EncodableKey;

    use crate::util::{env, init_logger};

    use super::*;

    #[tokio::test]
    #[ignore]
    async fn test_launch() {
        dotenv::dotenv().ok();
        std::env::set_var("RUST_LOG", "debug");
        init_logger().ok();
        let signer =
            Keypair::read_from_file(env("FUND_KEYPAIR_PATH")).unwrap();
        launch(
            "name".to_string(),
            "symbol".to_string(),
            "description".to_string(),
            &signer,
        )
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn test_push_meta_to_pump_ipfs() {
        let client = get_ipfs_client();
        let ipfs_meta = IPFSMetaForm::new(
            "name".to_string(),
            "symbol".to_string(),
            "description".to_string(),
        );
        let image = generate_random_image();
        let metadata_uri = push_meta_to_pump_ipfs(&client, &ipfs_meta, image)
            .await
            .unwrap();
        assert_eq!(metadata_uri.len(), 67);
    }

    #[tokio::test]
    async fn test_push_image_to_ipfs() {
        let client = get_ipfs_client();
        let image = generate_random_image();
        let res = push_image_to_ipfs(&client, image).await.unwrap();
        assert_eq!(res.len(), 46);
    }

    #[tokio::test]
    async fn test_push_meta_onto_ipfs() {
        let client = get_ipfs_client();
        let ipfs_meta = IPFSMetaForm::new(
            "name".to_string(),
            "symbol".to_string(),
            "description".to_string(),
        );
        let res = push_meta_onto_ipfs(&client, &ipfs_meta).await.unwrap();
        assert_eq!(res.len(), 46);
    }

    #[test]
    fn test_generate_mint() {
        let (pubkey, keypair) = generate_mint();
        assert_eq!(pubkey, keypair.pubkey());
    }

    #[test]
    fn test_get_bc_and_abc() {
        let mint =
            Pubkey::from_str("HUWAi6tdC3xW3gWG8G2W6HwhyNe9jf98m1ZRvoNtpump")
                .unwrap();
        let (bc, abc) = get_bc_and_abc(mint);
        assert!(bc != abc);
        assert_eq!(
            bc,
            Pubkey::from_str("DtfrDvHPqgDr85ySYBW4ZqnvFKxQ88taTGA7Nu6wQQFD")
                .unwrap()
        );
        assert_eq!(
            abc,
            Pubkey::from_str("HJcYNkA5EMcf2sqRdfkXktuXCDfxHcBTMSQY7G2dXxgo")
                .unwrap()
        );
    }

    #[test]
    fn test_instruction_data_format() {
        let name = "SCAMMER".to_string();
        let symbol = "SAHIL".to_string();
        let uri = "https://ipfs.io/ipfs/Qme6bpTaHjLafj3pdYvcFCAk6Kn33ckdWDEJxQDTYc95uF".to_string();

        let ix_data = PumpCreateTokenIx::new(name, symbol, uri);
        let mut buffer = Vec::new();
        ix_data.serialize(&mut buffer).unwrap();

        let expected = "181ec828051c0777070000005343414d4d455205000000534148494c4300000068747470733a2f2f697066732e696f2f697066732f516d653662705461486a4c61666a3370645976634643416b364b6e3333636b645744454a78514454596339357546";
        assert_eq!(hex::encode(buffer), expected);
    }
}
