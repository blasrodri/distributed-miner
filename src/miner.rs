use std::time::Duration;

use colored::*;
use ore_api::consts::BUS_ADDRESSES;
use solana_program::pubkey::Pubkey;
use solana_rpc_client::spinner;
use solana_sdk::{
    clock::Clock,
    commitment_config::CommitmentLevel,
    compute_budget::ComputeBudgetInstruction,
    instruction::Instruction,
    native_token::{lamports_to_sol, sol_to_lamports},
    signature::{Keypair, Signature},
    signer::Signer,
    sysvar,
    transaction::Transaction,
};

use solana_client::{
    client_error::{ClientError, ClientErrorKind, Result as ClientResult},
    rpc_client::RpcClient,
    rpc_config::RpcSendTransactionConfig,
};
use solana_transaction_status::{TransactionConfirmationStatus, UiTransactionEncoding};

const MIN_SOL_BALANCE: f64 = 0.005;

const RPC_RETRIES: usize = 0;
const _SIMULATION_RETRIES: usize = 4;
const GATEWAY_RETRIES: usize = 150;
const CONFIRM_RETRIES: usize = 1;

const CONFIRM_DELAY: u64 = 0;
const GATEWAY_DELAY: u64 = 300;

pub enum ComputeBudget {
    Dynamic,
    Fixed(u32),
}

pub fn send_and_confirm(
    client: &RpcClient,
    signer: &Keypair,
    ixs: &[Instruction],
    priority_fee: u64,
    compute_budget: ComputeBudget,
    skip_confirm: bool,
) -> ClientResult<Signature> {
    let progress_bar = spinner::new_progress_bar();

    // Return error, if balance is zero
    if let Ok(balance) = client.get_balance(&signer.pubkey()) {
        if balance <= sol_to_lamports(MIN_SOL_BALANCE) {
            panic!(
                "{} Insufficient balance: {} SOL\nPlease top up with at least {} SOL",
                "ERROR".bold().red(),
                lamports_to_sol(balance),
                MIN_SOL_BALANCE
            );
        }
    }

    // Set compute units
    let mut final_ixs = vec![];
    match compute_budget {
        ComputeBudget::Dynamic => {
            // TODO simulate
            final_ixs.push(ComputeBudgetInstruction::set_compute_unit_limit(1_400_000))
        }
        ComputeBudget::Fixed(cus) => {
            final_ixs.push(ComputeBudgetInstruction::set_compute_unit_limit(cus))
        }
    }
    final_ixs.push(ComputeBudgetInstruction::set_compute_unit_price(
        priority_fee,
    ));
    final_ixs.extend_from_slice(ixs);

    // Build tx
    let send_cfg = RpcSendTransactionConfig {
        skip_preflight: true,
        preflight_commitment: Some(CommitmentLevel::Confirmed),
        encoding: Some(UiTransactionEncoding::Base64),
        max_retries: Some(RPC_RETRIES),
        min_context_slot: None,
    };
    let mut tx = Transaction::new_with_payer(&final_ixs, Some(&signer.pubkey()));

    // Sign tx
    let (hash, _slot) = client
        .get_latest_blockhash_with_commitment(client.commitment())
        .unwrap();
    tx.sign(&[&signer], hash);

    // Submit tx
    let mut attempts = 0;
    loop {
        progress_bar.set_message(format!("Submitting transaction... (attempt {})", attempts));
        match client.send_transaction_with_config(&tx, send_cfg) {
            Ok(sig) => {
                // Skip confirmation
                if skip_confirm {
                    progress_bar.finish_with_message(format!("Sent: {}", sig));
                    return Ok(sig);
                }

                // Confirm the tx landed
                for _ in 0..CONFIRM_RETRIES {
                    std::thread::sleep(Duration::from_millis(CONFIRM_DELAY));
                    match client.get_signature_statuses(&[sig]) {
                        Ok(signature_statuses) => {
                            for status in signature_statuses.value {
                                if let Some(status) = status {
                                    if let Some(err) = status.err {
                                        progress_bar.finish_with_message(format!(
                                            "{}: {}",
                                            "ERROR".bold().red(),
                                            err
                                        ));
                                        return Err(ClientError {
                                            request: None,
                                            kind: ClientErrorKind::Custom(err.to_string()),
                                        });
                                    }
                                    if let Some(confirmation) = status.confirmation_status {
                                        match confirmation {
                                            TransactionConfirmationStatus::Processed => {}
                                            TransactionConfirmationStatus::Confirmed
                                            | TransactionConfirmationStatus::Finalized => {
                                                progress_bar.finish_with_message(format!(
                                                    "{} {}",
                                                    "OK".bold().green(),
                                                    sig
                                                ));
                                                return Ok(sig);
                                            }
                                        }
                                    }
                                }
                            }
                        }

                        // Handle confirmation errors
                        Err(err) => {
                            progress_bar.set_message(format!(
                                "{}: {}",
                                "ERROR".bold().red(),
                                err.kind().to_string()
                            ));
                        }
                    }
                }
            }

            // Handle submit errors
            Err(err) => {
                progress_bar.set_message(format!(
                    "{}: {}",
                    "ERROR".bold().red(),
                    err.kind().to_string()
                ));
            }
        }

        // Retry
        std::thread::sleep(Duration::from_millis(GATEWAY_DELAY));
        attempts += 1;
        if attempts > GATEWAY_RETRIES {
            progress_bar.finish_with_message(format!("{}: Max retries", "ERROR".bold().red()));
            return Err(ClientError {
                request: None,
                kind: ClientErrorKind::Custom("Max retries".into()),
            });
        }
    }
}
// TODO Pick a better strategy (avoid draining bus)
pub fn find_bus() -> Pubkey {
    BUS_ADDRESSES[2]
}

pub fn get_clock(client: &RpcClient) -> Clock {
    let data = client
        .get_account_data(&sysvar::clock::ID)
        .expect("Failed to get miner account");
    bincode::deserialize::<Clock>(&data).expect("Failed to deserialize clock")
}
