use borsh::{BorshDeserialize, BorshSerialize};
use drillx::{Hash, Solution};
use miner::{find_bus, send_and_confirm};
use ore_api::consts::{MIN_DIFFICULTY, ONE_MINUTE, PROOF};
use ore_api::state::Proof;
use ore_utils::AccountDeserialize;
use solana_rpc_client::rpc_client::RpcClient;
use solana_sdk::pubkey::Pubkey;
use solana_sdk::signature::Keypair;
use solana_sdk::signer::Signer;
use std::collections::{HashMap, HashSet};
use std::net::{TcpListener, TcpStream};
use std::sync::mpsc::{Receiver, SyncSender};
use std::thread::spawn;
use std::time::Instant;
use tungstenite::{accept, stream::MaybeTlsStream, Message, WebSocket};

pub mod miner;

pub struct MasterNode {
    rpc: RpcClient,
    keypair: Keypair,
    epoch_proofs: HashMap<Pubkey, Proof>,
    rx: Receiver<SubmittedSolutionEnum>,
    state: HashMap<Pubkey, InnerState>,
}

#[derive(Debug)]
struct InnerState {
    epoch_solutions: Vec<SubmittedSolution>,
    best_submitted_solution: SubmittedSolution,
    best_submitted_difficulty: u32,
}

#[derive(Debug, Clone, BorshDeserialize, BorshSerialize, PartialEq, Eq)]
pub enum SubmittedSolutionEnum {
    SubmittedSolution(SubmittedSolution),
    NewEpoch,
}

impl InnerState {
    fn new() -> Self {
        Self {
            epoch_solutions: vec![],
            best_submitted_difficulty: 0,
            best_submitted_solution: SubmittedSolution {
                staking_authority: Pubkey::default(),
                miner_authority: Pubkey::default(),
                solution: [0; 24],
            },
        }
    }
}

impl MasterNode {
    pub fn new(
        rpc: RpcClient,
        keypair: Keypair,
        proofs: HashMap<Pubkey, Proof>,
        rx: Receiver<SubmittedSolutionEnum>,
    ) -> Self {
        let state = proofs
            .keys()
            .into_iter()
            .map(|sa| (sa.clone(), InnerState::new()))
            .collect();
        Self {
            rpc,
            keypair,
            epoch_proofs: proofs,
            rx,
            state,
        }
    }

    pub fn run(&mut self) {
        loop {
            match self.rx.recv() {
                Ok(SubmittedSolutionEnum::SubmittedSolution(
                    submitted_solution @ SubmittedSolution { .. },
                )) => {
                    log::info!("processing new solution");
                    self.process_submitted_solution(submitted_solution);
                }
                Ok(SubmittedSolutionEnum::NewEpoch) => {
                    log::info!("processing new epoch");
                    self.process_new_epoch()
                }
                _ => panic!("wtf"),
            }
        }
    }

    fn process_submitted_solution(&mut self, submitted_solution: SubmittedSolution) {
        let SubmittedSolution {
            staking_authority,
            solution,
            ..
        } = submitted_solution;
        if let Some(inner_state) = self.state.get_mut(&staking_authority) {
            let digest = solution[0..16].try_into().unwrap();
            let nonce = solution[16..].try_into().unwrap();
            let solution = Solution::new(digest, nonce);
            let proof = self.epoch_proofs.get(&staking_authority).unwrap();
            log::info!("current challenge: {:?}", &proof.challenge);
            if !solution.is_valid(&proof.challenge) {
                log::error!("challenge not valid");
                return;
            }
            let hash = solution.to_hash();
            let difficulty = hash.difficulty();
            if dbg!(inner_state.best_submitted_difficulty) < difficulty {
                log::info!("Better difficulty submitted: {}", difficulty);
                inner_state.best_submitted_difficulty = difficulty;
                inner_state.best_submitted_solution = submitted_solution.clone();
            }
            inner_state.epoch_solutions.push(submitted_solution);
        }
    }

    fn process_new_epoch(&mut self) {
        // 1. submit best solution (if any)
        // 2. reset proofs
        // TODO: give rewards away

        for (staking_authority, inner_state) in &mut self.state {
            if inner_state.best_submitted_difficulty > MIN_DIFFICULTY {
                let digest = inner_state.best_submitted_solution.solution[0..16]
                    .try_into()
                    .unwrap();
                let nonce = inner_state.best_submitted_solution.solution[16..]
                    .try_into()
                    .unwrap();
                let solution = Solution::new(digest, nonce);
                let ixs = vec![ore_api::instruction::mine(
                    self.keypair.pubkey(),
                    *staking_authority,
                    find_bus(),
                    solution,
                )];
                // todo: in parallel
                let result = send_and_confirm(&self.rpc, &self.keypair, &ixs, false).unwrap();
                log::info!("Signature: {result}");
            }
            inner_state.best_submitted_difficulty = 0;
            inner_state.epoch_solutions.clear();

            // get new proof
            let new_proof = get_proof(&self.rpc, *staking_authority);
            log::info!("new challenge: {:?}", new_proof.challenge);
            self.epoch_proofs.insert(*staking_authority, new_proof);
        }
    }
}

pub fn start_websocket_server(host: String, solution_tx: SyncSender<SubmittedSolutionEnum>) {
    let server = TcpListener::bind(host.as_str()).unwrap();
    for stream in server.incoming() {
        let solution_tx = solution_tx.clone();
        spawn(move || {
            let mut websocket = accept(stream.unwrap()).unwrap();
            loop {
                let msg = websocket.read();
                if msg.is_err() {
                    continue;
                }
                let msg = msg.unwrap();
                // We do not want to send back ping/pong messages.
                if msg.is_binary() || msg.is_text() {
                    if let Ok(
                        submitted_solution @ SubmittedSolutionEnum::SubmittedSolution { .. },
                    ) = borsh::from_slice(&msg.into_data())
                    {
                        solution_tx.send(submitted_solution).unwrap();
                    }
                }
            }
        });
    }
}

type Socket = WebSocket<MaybeTlsStream<TcpStream>>;
type Challenge = [u8; 32];

#[derive(Debug, Clone, BorshDeserialize, BorshSerialize)]
pub struct ChallengeInput {
    pub challenge: Challenge,
    remaining_time: u64,
}

#[derive(Debug, Clone, BorshDeserialize, BorshSerialize, PartialEq, Eq)]
pub struct SubmittedSolution {
    pub staking_authority: Pubkey,
    pub miner_authority: Pubkey,
    pub solution: [u8; 24], // 16 digest - 8 nonce
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
            remaining_time: 20,
        }
    }
    pub fn send_solution(socket: &mut Socket, solution: Vec<u8>) {
        let msg = Message::binary(solution);
        socket.write(msg).unwrap();
        socket.flush().unwrap();
    }
}

pub fn get_proof(client: &RpcClient, authority: Pubkey) -> Proof {
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

pub fn get_hash(challenge: ChallengeInput) -> (Hash, u64) {
    loop {
        let threads = 1;
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
                        let mut best_nonce = 0;
                        loop {
                            // Create hash
                            if let Ok(hx) = drillx::hash_with_memory(
                                &mut memory,
                                &challenge,
                                &nonce.to_le_bytes(),
                            ) {
                                let solution = Solution::new(hx.d, nonce.to_le_bytes());
                                if solution.is_valid(&challenge) {
                                    let difficulty = hx.difficulty();
                                    if difficulty.gt(&best_difficulty) {
                                        best_difficulty = difficulty;
                                        best_hash = hx;
                                        best_nonce = nonce;
                                    }
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
                        (best_difficulty, best_hash, best_nonce)
                    }
                })
            })
            .collect();

        // Join handles and return best nonce
        let mut best_difficulty = 0;
        let mut best_nonce = 0;
        let mut best_hash = Hash::default();
        for h in handles {
            if let Ok((difficulty, hash, nonce)) = h.join() {
                if difficulty > best_difficulty {
                    best_difficulty = difficulty;
                    best_hash = hash;
                    best_nonce = nonce;
                }
            }
        }

        println!("diff: {best_difficulty}");
        return (best_hash, best_nonce);
    }
}
// drillx::hash_with_memory(&mut memory, challenge, nonce);
