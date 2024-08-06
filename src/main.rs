use std::{
    str::FromStr,
    sync::mpsc::sync_channel,
    thread::{self, sleep, spawn},
    time::Duration,
};

use distributed_drillx::{
    get_hash, get_proof, miner::get_clock, start_websocket_server, MasterNode, NodeHashComputer,
    SubmittedSolution, SubmittedSolutionEnum,
};
use solana_rpc_client::rpc_client::RpcClient;
use solana_sdk::{
    commitment_config::CommitmentConfig,
    pubkey::Pubkey,
    signature::Keypair,
    signer::{EncodableKey, Signer},
};
use structopt::StructOpt;

fn main() {
    env_logger::init();
    let opt = NodeType::from_args();

    let (tx, rx) = sync_channel(1_000);
    match opt {
        NodeType::Master {
            host,
            keypair: keypair_path,
            rpc_url,
            priority_fees,
        } => {
            let rpc_client: RpcClient =
                RpcClient::new_with_commitment(rpc_url.clone(), CommitmentConfig::confirmed());
            let keypair: Keypair =
                Keypair::read_from_file(keypair_path).expect("could not read keypair");
            let tx_cloned = tx.clone();
            thread::spawn(move || start_websocket_server(host, tx_cloned));
            // TODO: load staking authorities from a file or whatever
            let proof = get_proof(&rpc_client, keypair.pubkey());
            log::info!("{:?}", proof.last_hash_at);
            let staking_authority = keypair.pubkey();

            let mut master_node = MasterNode::new(
                rpc_client,
                keypair,
                [(staking_authority.clone(), proof.challenge)]
                    .into_iter()
                    .collect(),
                rx,
                priority_fees,
            );
            // spawn new epoch thread
            spawn(move || {
                let rpc_client: RpcClient =
                    RpcClient::new_with_commitment(rpc_url, CommitmentConfig::confirmed());
                loop {
                    let proof = get_proof(&rpc_client, staking_authority.clone());
                    let clock = get_clock(&rpc_client);

                    let next_cutoff = dbg!(proof.last_hash_at)
                        .saturating_add(60)
                        .saturating_sub(1 as i64)
                        .saturating_sub(dbg!(clock.unix_timestamp))
                        .max(20) as u64;
                    log::info!("Next cutoff in {next_cutoff} seconds");
                    sleep(Duration::from_secs(next_cutoff));
                    tx.send(distributed_drillx::SubmittedSolutionEnum::NewEpoch(
                        staking_authority,
                    ))
                    .unwrap();
                    log::info!("New epoch submitted");
                }
            });
            master_node.run();
        }
        NodeType::Node {
            master,
            miner_authority,
            rpc_url,
            ..
        } => {
            let rpc_client: RpcClient =
                RpcClient::new_with_commitment(rpc_url, CommitmentConfig::confirmed());
            let staker_authority =
                Pubkey::from_str(&miner_authority).expect("could not load miner authority");
            let mut socket = NodeHashComputer::connect(master).unwrap();
            // move this to its own function
            loop {
                let challenge = NodeHashComputer::receive_challenge(&rpc_client, staker_authority);
                log::info!("challenge: {:?}", challenge);

                let (solution_hash, nonce) = get_hash(challenge.clone());
                let solution =
                    [solution_hash.d.as_slice(), nonce.to_le_bytes().as_slice()].concat();
                // let s = Solution::new(solution_hash.d, nonce.to_le_bytes());
                // assert!(s.is_valid(&challenge.challenge));
                // assert!(solution.len() == 24);
                let submitted_solution =
                    SubmittedSolutionEnum::SubmittedSolution(SubmittedSolution {
                        staking_authority: staker_authority,
                        miner_authority: staker_authority,
                        solution: solution.try_into().unwrap(),
                    });
                NodeHashComputer::send_solution(
                    &mut socket,
                    borsh::to_vec(&submitted_solution).unwrap(),
                );
            }
        }
    }
}

#[derive(Debug, StructOpt)]
enum NodeType {
    Master {
        #[structopt(short = "h", long = "host", default_value = "127.0.0.1")]
        host: String,
        #[structopt(
            short = "k",
            long = "keypair",
            default_value = "/Users/blasrodriguezgarciairizar/.config/solana/id.json"
        )]
        keypair: String,
        #[structopt(
            short = "r",
            long = "rpc_url",
            default_value = "http://api.mainnet-beta.solana.com"
        )]
        rpc_url: String,
        #[structopt(short = "p", long = "priority_fees", default_value = "0")]
        priority_fees: u64,
    },
    Node {
        #[structopt(short = "m", long = "master", default_value = "127.0.0.1")]
        master: String,
        #[structopt(
            short = "m",
            long = "miner_authority",
            default_value = "9kQxYE42uPunfSQE4925mNZ7nV1REXtCPg944UfVcRLZ"
        )]
        miner_authority: String,
        #[structopt(
            short = "r",
            long = "rpc_url",
            default_value = "http://api.mainnet-beta.solana.com"
        )]
        rpc_url: String,
    },
}
