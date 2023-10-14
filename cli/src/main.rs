use std::collections::HashSet;
use std::error::Error;
use std::net::IpAddr;
use std::process;
use std::{
    path::PathBuf,
    process::exit,
    str::FromStr,
    sync::{Arc, Mutex},
};

use clap::Parser;
use client::{Client, ClientBuilder};
use common::utils::hex_str_to_bytes;
use config::{CliConfig, Config};
use dirs::home_dir;
use ethers::{
    core::types::{Address, Block, BlockId, Transaction, H256},
    providers::{Http, Middleware, Provider},
    // signers::Wallet,
    // trie::{MerklePatriciaTrie, Trie},
};
use futures::executor::block_on;
use tracing::{error, info};
use tracing_subscriber::filter::{EnvFilter, LevelFilter};
use tracing_subscriber::FmtSubscriber;
// use ethers::types::transaction;
use eyre::{Result};

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let env_filter = EnvFilter::builder()
        .with_default_directive(LevelFilter::INFO.into())
        .from_env()
        .expect("invalid env filter");

    let subscriber = FmtSubscriber::builder()
        .with_env_filter(env_filter)
        .finish();

    tracing::subscriber::set_global_default(subscriber).expect("subsriber set failed");

    let provider_url = std::env::var("GOERLI_EXECUTION_RPC")
        .unwrap_or_else(|_| "http://127.0.0.1:8545".to_string());
    let provider = Provider::<Http>::try_from(provider_url)?;
    // tracing::debug!("provider {:#?}", provider);
    let block_number = 9751183.into(); // Replace with the desired block number
                                       // Fetch all transactions within the specified block
    let transactions = fetch_all_transactions(provider.clone(), block_number).await;
    tracing::info!("transactions {:#?}", transactions);

    // Fetch the block data, including the state root
    let block = fetch_block_data(provider.clone(), block_number).await;
    tracing::info!("block state root {:#?}", block.state_root);

    let addresses = transactions
        .iter()
        .flat_map(|tx| {
            let from = tx.from;
            let to = tx.to;
            vec![from, to.unwrap_or_default()]
        })
        .collect::<Vec<_>>();
    // Remove duplicate addresses
    let addresses_deduped = dedup(&addresses);
    tracing::info!("addresses {:#?}", addresses_deduped);

    let config = get_config();

    // Create the Helios client with the specified target addresses
    let mut client = match ClientBuilder::new().config(config).build() {
        Ok(client) => client,
        Err(err) => {
            error!(target: "helios::runner", error = %err);
            exit(1);
        }
    };

    if let Err(err) = client.start().await {
        error!(target: "helios::runner", error = %err);
        exit(1);
    }

    register_shutdown_handler(client);
    std::future::pending().await
}

fn register_shutdown_handler(client: Client) {
    let client = Arc::new(client);
    let shutdown_counter = Arc::new(Mutex::new(0));

    ctrlc::set_handler(move || {
        let mut counter = shutdown_counter.lock().unwrap();
        *counter += 1;

        let counter_value = *counter;

        if counter_value == 3 {
            info!(target: "helios::runner", "forced shutdown");
            exit(0);
        }

        info!(
            target: "helios::runner",
            "shutting down... press ctrl-c {} more times to force quit",
            3 - counter_value
        );

        if counter_value == 1 {
            let client = client.clone();
            std::thread::spawn(move || {
                block_on(client.shutdown());
                exit(0);
            });
        }
    })
    .expect("could not register shutdown handler");
}

fn get_config() -> Config {
    let cli = Cli::parse();

    let config_path = home_dir().unwrap().join(".helios/helios.toml");

    let cli_config = cli.as_cli_config();

    Config::from_file(&config_path, &cli.network, &cli_config)
}
//sogol added:
// Fetch the block data, including the state root
// let block_number_to_fetch = 12345; // Replace with the desired block number
// let block_data = fetch_block_data(&provider_url, block_number_to_fetch).await?;

// // Iterate through accounts and fetch their state roots
// for account_address in get_all_accounts(&block_data.state_root) {
//     let state_root = fetch_state_root(&block_data.state_root, &account_address).await?;
//     // Process or store the state root as needed
//     println!("Account: {:?}, State Root: {:?}", account_address, state_root);
// }

async fn fetch_block_data(provider: Provider<Http>, block_number: BlockId) -> Block<H256> {
    let block;
    match provider.get_block(block_number).await {
        Ok(x) => {
            if let Some(_block) = x {
                block = _block;
            } else {
                tracing::debug!("Received empty block");
                process::exit(1);
            }
        }
        Err(e) => {
            tracing::debug!("Unable to get block {:#?}", e);
            process::exit(1);
        }
    }
    // println!("block {:?}", block);

    block
}

async fn fetch_all_transactions(
    provider: Provider<Http>,
    block_number: BlockId,
) -> Vec<Transaction> {
    let block_with_txs;
    match provider.get_block_with_txs(block_number).await {
        Ok(x) => {
            if let Some(_block_with_txs) = x {
                block_with_txs = _block_with_txs;
            } else {
                tracing::debug!("Received empty block with transactions");
                process::exit(1);
            }
        }
        Err(e) => {
            tracing::debug!("Unable to get block with transactions {:#?}", e);
            process::exit(1);
        }
    }
    let transactions: Vec<Transaction> = block_with_txs.transactions;

    transactions
}

fn dedup(vs: &[Address]) -> Vec<Address> {
    let hs = vs.iter().cloned().collect::<HashSet<Address>>();

    hs.into_iter().collect()
}

#[derive(Parser)]
#[clap(version, about)]
/// Helios is a fast, secure, and portable light client for Ethereum
struct Cli {
    #[clap(short, long, default_value = "mainnet")]
    network: String,
    #[clap(short = 'b', long, env)]
    rpc_bind_ip: Option<IpAddr>,
    #[clap(short = 'p', long, env)]
    rpc_port: Option<u16>,
    #[clap(short = 'w', long, env)]
    checkpoint: Option<String>,
    #[clap(short, long, env)]
    execution_rpc: Option<String>,
    #[clap(short, long, env)]
    consensus_rpc: Option<String>,
    #[clap(short, long, env)]
    data_dir: Option<String>,
    #[clap(short = 'f', long, env)]
    fallback: Option<String>,
    #[clap(short = 'l', long, env)]
    load_external_fallback: bool,
    #[clap(short = 's', long, env)]
    strict_checkpoint_age: bool,
    #[clap(short = 'a', long, env)]
    target_addresses: Option<Vec<String>>,
}

impl Cli {
    fn as_cli_config(&self) -> CliConfig {
        let checkpoint = self
            .checkpoint
            .as_ref()
            .map(|c| hex_str_to_bytes(c).expect("invalid checkpoint"));
        CliConfig {
            checkpoint,
            execution_rpc: self.execution_rpc.clone(),
            consensus_rpc: self.consensus_rpc.clone(),
            data_dir: self.get_data_dir(),
            rpc_bind_ip: self.rpc_bind_ip,
            rpc_port: self.rpc_port,
            fallback: self.fallback.clone(),
            load_external_fallback: self.load_external_fallback,
            strict_checkpoint_age: self.strict_checkpoint_age,
            target_addresses: self.target_addresses.clone(),
        }
    }

    fn get_data_dir(&self) -> PathBuf {
        if let Some(dir) = &self.data_dir {
            PathBuf::from_str(dir).expect("cannot find data dir")
        } else {
            home_dir()
                .unwrap()
                .join(format!(".helios/data/{}", self.network))
        }
    }
}
