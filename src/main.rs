use std::str::FromStr;

use distributed_drillx::{get_hash, MasterNode, NodeHashComputer};
use solana_rpc_client::rpc_client::RpcClient;
use solana_sdk::{
    commitment_config::CommitmentConfig, pubkey::Pubkey, signature::Keypair, signer::EncodableKey,
};
use structopt::StructOpt;

fn main() {
    env_logger::init();
    let opt = NodeType::from_args();

    match opt {
        NodeType::Master { host, keypair } => {
            let keypair = Keypair::read_from_file(keypair).expect("could not read keypair");
            MasterNode::start_websocket_server(host);
        }
        NodeType::Node {
            master,
            keypair,
            miner_authority,
        } => {
            let keypair = Keypair::read_from_file(keypair).expect("could not read keypair");
            let cluster = "https://api.devnet.solana.com";
            let rpc_client = RpcClient::new_with_commitment(cluster, CommitmentConfig::confirmed());

            let staker_authority =
                Pubkey::from_str(&miner_authority).expect("could not load miner authority");
            let mut socket = NodeHashComputer::connect(master).unwrap();
            // move this to its own function
            loop {
                let challenge = NodeHashComputer::receive_challenge(&rpc_client, staker_authority);

                let solution = get_hash(challenge);
                let solution = [solution.d.as_slice(), solution.h.as_slice()].concat();
                NodeHashComputer::send_solution(&mut socket, solution);
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
    },
    Node {
        #[structopt(short = "m", long = "master", default_value = "127.0.0.1")]
        master: String,
        #[structopt(
            short = "k",
            long = "keypair",
            default_value = "/Users/blasrodriguezgarciairizar/.config/solana/id.json"
        )]
        keypair: String,
        #[structopt(
            short = "m",
            long = "miner_authority",
            default_value = "9kQxYE42uPunfSQE4925mNZ7nV1REXtCPg944UfVcRLZ"
        )]
        miner_authority: String,
    },
}
