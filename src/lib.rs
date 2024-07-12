use borsh::{BorshDeserialize, BorshSerialize};
use drillx::Hash;
use ore_api::consts::{ONE_MINUTE, PROOF};
use ore_api::state::Proof;
use ore_utils::AccountDeserialize;
use solana_rpc_client::rpc_client::RpcClient;
use solana_sdk::pubkey::Pubkey;
use std::net::{TcpListener, TcpStream};
use std::thread::spawn;
use std::time::Instant;
use tungstenite::{accept, stream::MaybeTlsStream, Message, WebSocket};

pub struct MasterNode {}

impl MasterNode {
    pub fn start_websocket_server(host: String) {
        let server = TcpListener::bind(host.as_str()).unwrap();
        for stream in server.incoming() {
            spawn(move || {
                let mut websocket = accept(stream.unwrap()).unwrap();
                loop {
                    let msg = websocket.read();
                    if msg.is_err() {
                        continue;
                    }
                    let msg = msg.unwrap();
                    dbg!(&msg);
                    // We do not want to send back ping/pong messages.
                    if msg.is_binary() || msg.is_text() {
                        if let Ok(ChallengeRequest { .. }) = borsh::from_slice(&msg.into_data()) {
                            let remainig_time = 10u64;
                            let challenge = [1; 32];
                            let msg = Self::pack_msg(challenge, remainig_time);
                            websocket.send(msg).unwrap();
                        }
                    }
                }
            });
        }
    }

    fn pack_msg(challenge: [u8; 32], remaining_time: u64) -> Message {
        let msg = borsh::to_vec(&ChallengeInput {
            challenge,
            remaining_time,
        })
        .unwrap();
        Message::binary(msg)
    }
}

type Socket = WebSocket<MaybeTlsStream<TcpStream>>;
type Challenge = [u8; 32];

#[derive(Clone, BorshDeserialize, BorshSerialize)]
pub struct ChallengeInput {
    challenge: Challenge,
    remaining_time: u64,
}

#[derive(Clone, BorshDeserialize, BorshSerialize)]
pub struct ChallengeRequest {
    staker_authority: Pubkey,
}

pub struct NodeHashComputer {}

impl NodeHashComputer {
    pub fn connect(host: String) -> Option<Socket> {
        let (socket, _) = tungstenite::connect(&host).expect("Can't connect");
        log::info!("Connected to the server");
        return Some(socket);
    }

    pub fn receive_challenge(rpc_client: &RpcClient, staker_authority: Pubkey) -> ChallengeInput {
        // ore_cli::utils::
        let proof = get_proof(rpc_client, staker_authority);
        ChallengeInput {
            challenge: proof.challenge,
            // remaining_time: proof.last_stake_at.saturating_add(ONE_MINUTE) as _,
            remaining_time: 10,
        }
    }
    pub fn send_solution(socket: &mut Socket, solution: Vec<u8>) {
        let msg = Message::binary(solution);
        socket.write(msg).unwrap();
        socket.flush().unwrap();
    }
}

fn get_proof(client: &RpcClient, authority: Pubkey) -> Proof {
    let proof_address = proof_pubkey(authority);
    let data = client
        .get_account_data(&proof_address)
        // .await
        .expect("Failed to get miner account");
    *Proof::try_from_bytes(&data).expect("Failed to parse miner account")
}

pub fn proof_pubkey(authority: Pubkey) -> Pubkey {
    Pubkey::find_program_address(&[PROOF, authority.as_ref()], &ore_api::ID).0
}

pub fn get_hash(challenge: ChallengeInput) -> Hash {
    loop {
        let threads = 10;
        let handles: Vec<_> = (0..threads)
            .map(|i| {
                std::thread::spawn({
                    let ChallengeInput {
                        challenge,
                        remaining_time,
                    } = challenge.clone();
                    let timer = Instant::now();
                    let mut memory = drillx::equix::SolverMemory::new();
                    move || {
                        let mut nonce = u64::MAX.saturating_div(threads).saturating_mul(i);
                        let mut best_difficulty = 0;
                        let mut best_hash = Hash::default();
                        loop {
                            // Create hash
                            if let Ok(hx) = drillx::hash_with_memory(
                                &mut memory,
                                &challenge,
                                &nonce.to_le_bytes(),
                            ) {
                                let difficulty = hx.difficulty();
                                if difficulty.gt(&best_difficulty) {
                                    best_difficulty = difficulty;
                                    best_hash = hx;
                                }
                            }

                            // Exit if time has elapsed
                            if timer.elapsed().as_secs().ge(&remaining_time) {
                                break;
                            }
                            // Increment nonce
                            nonce += 1;
                        }

                        // Return the best nonce
                        (best_difficulty, best_hash)
                    }
                })
            })
            .collect();

        // Join handles and return best nonce
        let mut best_difficulty = 0;
        let mut best_hash = Hash::default();
        for h in handles {
            if let Ok((difficulty, hash)) = h.join() {
                if difficulty > best_difficulty {
                    best_difficulty = difficulty;
                    best_hash = hash;
                }
            }
        }

        println!("diff: {best_difficulty}");
        return best_hash;
    }
}
// drillx::hash_with_memory(&mut memory, challenge, nonce);
