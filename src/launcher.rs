use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use borsh::{BorshDeserialize, BorshSerialize};
use chrono::Utc;
use rand::Rng;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use solana_sdk::{
    instruction::{AccountMeta, Instruction},
    pubkey::Pubkey,
    signature::Keypair,
    signer::Signer,
};
use std::str::FromStr;

use crate::constants::{
    ASSOCIATED_TOKEN_PROGRAM, EVENT_AUTHORITY, PUMP_FUN_MINT_AUTHORITY,
    PUMP_FUN_PROGRAM, PUMP_GLOBAL_ADDRESS, RENT_PROGRAM, SYSTEM_PROGRAM_ID,
    TOKEN_PROGRAM,
};
use crate::util::make_compute_budget_ixs;

pub const MPL_TOKEN_METADATA: &str =
    "metaqbxxUerdq28cj1RbAWkYQm3ybzjb6a8bt518x1s";
// this might be derived
pub const METADATA: &str = "GgrH3ApmK1SYJVZNEuUavbZQx4Yt8WoBz3tkRuLKwj45";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IPFSMeta {
    pub name: String,
    pub symbol: String,
    pub description: String,
    pub image: String,
    pub show_name: bool,
    pub created_on: String,
    pub twitter: String,
    pub website: String,
}

impl IPFSMeta {
    pub fn new(
        name: String,
        symbol: String,
        description: String,
        image: String,
        show_name: bool,
    ) -> Self {
        Self {
            name,
            symbol,
            description,
            image,
            show_name,
            created_on: Utc::now().to_rfc3339(),
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

#[derive(BorshSerialize, BorshDeserialize)]
pub struct PumpCreateTokenIx {
    pub name: String,
    pub symbol: String,
    pub uri: String,
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
    ipfs_meta: &IPFSMeta,
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

    Ok(res["Hash"].as_str().unwrap().to_string())
}

pub async fn launch(
    name: String,
    symbol: String,
    description: String,
    signer: &Keypair,
) -> Result<Vec<Instruction>, Box<dyn std::error::Error>> {
    let mut ixs = vec![];

    // Add compute budget instructions
    ixs.append(&mut make_compute_budget_ixs(542850, 250000));

    let image = generate_random_image();
    // Generate and push random image to IPFS
    let client = get_ipfs_client();
    let image_uri = push_image_to_ipfs(&client, image).await?;

    // Create and push metadata to IPFS
    let ipfs_meta = IPFSMeta::new(
        name.clone(),
        symbol.clone(),
        description.clone(),
        image_uri.clone(),
        true,
    );
    let metadata_uri = push_meta_onto_ipfs(&client, &ipfs_meta).await?;

    ixs.push(_make_create_token_ix(
        name,
        symbol,
        metadata_uri,
        Pubkey::default(),
        Pubkey::default(),
        Pubkey::default(),
        Pubkey::default(),
        signer.pubkey(),
    ));

    Ok(ixs)
}

pub fn _make_create_token_ix(
    name: String,
    symbol: String,
    metadata_uri: String,
    metadata: Pubkey,
    mint: Pubkey,
    bonding_curve: Pubkey,
    associated_bonding_curve: Pubkey,
    user: Pubkey,
) -> Instruction {
    // Construct the instruction data
    let instruction_data = PumpCreateTokenIx {
        name,
        symbol,
        uri: metadata_uri,
    };

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
    ];

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

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_push_image_to_ipfs() {
        let client = get_ipfs_client();
        let image = generate_random_image();
        let res = push_image_to_ipfs(&client, image).await.unwrap();
        println!("res: {}", res);
        assert_eq!(res.len(), 46);
        panic!();
    }

    #[tokio::test]
    async fn test_push_meta_onto_ipfs() {
        let client = get_ipfs_client();
        let ipfs_meta = super::IPFSMeta::new(
            "name".to_string(),
            "symbol".to_string(),
            "description".to_string(),
            "image".to_string(),
            true,
        );
        let res = super::push_meta_onto_ipfs(&client, &ipfs_meta)
            .await
            .unwrap();
        assert_eq!(res.len(), 46);
    }
}
