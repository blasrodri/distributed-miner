use std::time::Duration;

use drillx::Solution;
use ore_api::consts::{BUS_ADDRESSES, BUS_COUNT, EPOCH_DURATION};
use solana_program::pubkey::Pubkey;
use solana_rpc_client::spinner;
use solana_sdk::{
    clock::Clock,
    commitment_config::CommitmentLevel,
    compute_budget::ComputeBudgetInstruction,
    instruction::Instruction,
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

pub async fn mine(signer: Keypair, solution: Solution) {
    // Submit most difficult hash
    let mut ixs = vec![];
    ixs.push(ore_api::instruction::mine(
        signer.pubkey(),
        find_bus(),
        solution,
    ));
}

const RPC_RETRIES: usize = 10;

pub fn send_and_confirm(
    rpc_client: &RpcClient,
    signer: &Keypair,
    ixs: &[Instruction],
    skip_confirm: bool,
) -> ClientResult<Signature> {
    let progress_bar = spinner::new_progress_bar();

    // Set compute units
    let mut final_ixs = vec![];
    final_ixs.push(ComputeBudgetInstruction::set_compute_unit_limit(1_400_000));
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
    let (hash, _slot) = rpc_client
        .get_latest_blockhash_with_commitment(rpc_client.commitment())
        .unwrap();
    tx.sign(&[&signer], hash);

    // Submit tx
    let mut attempts = 0;
    loop {
        progress_bar.set_message(format!("Submitting transaction... (attempt {})", attempts));
        match rpc_client.send_transaction_with_config(&tx, send_cfg) {
            Ok(sig) => {
                // Skip confirmation
                if skip_confirm {
                    progress_bar.finish_with_message(format!("Sent: {}", sig));
                    return Ok(sig);
                }

                // Confirm the tx landed
                for _ in 0..RPC_RETRIES {
                    std::thread::sleep(Duration::from_millis(100));
                    match rpc_client.get_signature_statuses(&[sig]) {
                        Ok(signature_statuses) => {
                            for status in signature_statuses.value {
                                if let Some(status) = status {
                                    if let Some(err) = status.err {
                                        progress_bar.set_message(format!("Error: {}", err));
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
                                                return Ok(sig);
                                            }
                                        }
                                    }
                                }
                            }
                        }

                        // Handle confirmation errors
                        Err(_err) => {}
                    }
                }
            }

            // Handle submit errors
            Err(err) => {}
        }

        // Retry
        std::thread::sleep(Duration::from_millis(100));
        attempts += 1;
        if attempts > 10 {
            // progress_bar.finish_with_message(format!("{}: Max retries", "ERROR".bold().red()));
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
