use std::sync::Arc;

use ethers::prelude::ContractCall;
use ethers::providers::Middleware;
use ethers::types::{Address, TransactionReceipt, TransactionRequest};
use rocket::tokio::{self, sync::oneshot};
use sp_core::H256;
use tokio::sync::mpsc::UnboundedReceiver;
use webb::evm::contract::protocol_solidity::erc20_preset_minter_pauser::ERC20PresetMinterPauserContract;
use webb::evm::ethers;
use webb::evm::ethers::types::U256;
use webb::substrate::subxt::utils::{AccountId32, MultiAddress};
use webb::substrate::subxt::{tx::PairSigner, OnlineClient, PolkadotConfig};
use webb::substrate::tangle_runtime::api as RuntimeApi;

use crate::error::Error;

use super::types::{Transaction, TxResult};

pub struct TransactionProcessingSystem {
    rx_receiver: UnboundedReceiver<Transaction>,
}

impl TransactionProcessingSystem {
    pub fn new(rx_receiver: UnboundedReceiver<Transaction>) -> Self {
        Self { rx_receiver }
    }

    pub fn run(mut self) {
        tokio::spawn(async move {
            println!("Transaction processing system started");
            while let Some(transaction) = self.rx_receiver.recv().await {
                match transaction {
                    Transaction::Evm {
                        provider,
                        to,
                        amount,
                        token_address,
                        result_sender,
                    } => {
                        let res = handle_evm_tx(
                            provider,
                            to,
                            amount,
                            token_address,
                            result_sender,
                        )
                        .await;
                        if let Err(e) = res {
                            eprintln!("Error processing EVM transaction: {e}");
                        }
                    }
                    Transaction::Substrate {
                        api,
                        to,
                        amount,
                        asset_id,
                        signer,
                        result_sender,
                    } => {
                        let res = handle_substrate_tx(
                            api,
                            to,
                            amount,
                            asset_id,
                            signer,
                            result_sender,
                        )
                        .await;
                        if let Err(e) = res {
                            eprintln!(
                                "Error processing Substrate transaction: {e}"
                            );
                        }
                    }
                }
            }
            eprintln!("Transaction processing system stopped");
        });
    }
}

async fn handle_evm_tx<M: Middleware>(
    provider: M,
    to: Address,
    amount: U256,
    token_address: Option<Address>,
    result_sender: oneshot::Sender<Result<TxResult, Error>>,
) -> Result<TransactionReceipt, Error> {
    match token_address {
        Some(token_address) => {
            handle_evm_token_tx(
                provider,
                to,
                amount,
                token_address,
                result_sender,
            )
            .await
        }
        None => handle_evm_native_tx(provider, to, amount, result_sender).await,
    }
}

async fn handle_evm_native_tx<M: Middleware>(
    provider: M,
    to: Address,
    amount: U256,
    result_sender: oneshot::Sender<Result<TxResult, Error>>,
) -> Result<TransactionReceipt, Error> {
    // Craft the tx
    let accounts = provider
        .get_accounts()
        .await
        .map_err(|e| Error::Custom(e.to_string()))?;
    let tx = TransactionRequest::new()
        .to(to)
        .value(amount)
        .from(accounts[0]);

    // Broadcast it via the eth_sendTransaction API
    let tx_receipt = provider
        .send_transaction(tx, None)
        .await
        .map_err(|e| Error::Custom(e.to_string()))?
        .await
        .map_err(|e| Error::Custom(e.to_string()))?;
    match tx_receipt {
        Some(receipt) => {
            result_sender
                .send(Ok(TxResult::Evm(receipt.clone())))
                .map_err(|e| {
                    Error::Custom(format!("Failed to send receipt: {:?}", e))
                })?;
            Ok(receipt)
        }
        None => {
            result_sender
                .send(Err(Error::Custom(
                    "Failed to send transaction".to_string(),
                )))
                .map_err(|e| {
                    Error::Custom(format!(
                        "Failed to send transaction: {:?}",
                        e
                    ))
                })?;
            Err(Error::Custom("Failed to send transaction".to_string()))
        }
    }
}

async fn handle_evm_token_tx<M: Middleware>(
    provider: M,
    to: Address,
    amount: U256,
    token_address: Address,
    result_sender: oneshot::Sender<Result<TxResult, Error>>,
) -> Result<TransactionReceipt, Error> {
    let has_signer = provider.is_signer().await;
    assert!(has_signer, "Provider must have signer");
    let contract =
        ERC20PresetMinterPauserContract::new(token_address, Arc::new(provider));

    // Fetch the decimals used by the contract so we can compute the decimal amount to send.
    let decimals = contract.decimals().call().await.map_err(|e| {
        Error::Custom(format!("Failed to fetch decimals: {:?}", e))
    })?;
    let decimal_amount = amount * U256::exp10(decimals as usize);

    // Transfer the desired amount of tokens to the `to_address`
    let tx: ContractCall<M, _> = contract.transfer(to, decimal_amount).legacy();
    let pending_tx = tx
        .send()
        .await
        .map_err(|e| Error::Custom(format!("Failed to send tx: {:?}", e)))?;
    match pending_tx
        .await
        .map_err(|e| Error::Custom(format!("Failed to await tx: {:?}", e)))?
    {
        Some(receipt) => {
            result_sender
                .send(Ok(TxResult::Evm(receipt.clone())))
                .map_err(|e| {
                    Error::Custom(format!("Failed to send receipt: {:?}", e))
                })?;
            Ok(receipt)
        }
        None => {
            result_sender
                .send(Err(Error::Custom(
                    "Failed to send transaction".to_string(),
                )))
                .map_err(|e| {
                    Error::Custom(format!(
                        "Failed to send transaction: {:?}",
                        e
                    ))
                })?;
            Err(Error::Custom("Failed to send transaction".to_string()))
        }
    }
}

async fn handle_substrate_tx(
    api: OnlineClient<PolkadotConfig>,
    to: AccountId32,
    amount: u128,
    asset_id: Option<u32>,
    signer: sp_core::sr25519::Pair,
    result_sender: oneshot::Sender<Result<TxResult, Error>>,
) -> Result<H256, Error> {
    match asset_id {
        Some(asset_id) => {
            handle_substrate_token_tx(
                api,
                to,
                amount,
                asset_id,
                signer,
                result_sender,
            )
            .await
        }
        None => {
            handle_substrate_native_tx(api, to, amount, signer, result_sender)
                .await
        }
    }
}

async fn handle_substrate_native_tx(
    api: OnlineClient<PolkadotConfig>,
    to: AccountId32,
    amount: u128,
    signer: sp_core::sr25519::Pair,
    result_sender: oneshot::Sender<Result<TxResult, Error>>,
) -> Result<H256, Error> {
    let to_address = MultiAddress::Id(to.clone());
    let balance_transfer_tx =
        RuntimeApi::tx().balances().transfer(to_address, amount);

    // Sign and submit the extrinsic.
    let tx_result = api
        .tx()
        .sign_and_submit_then_watch_default(
            &balance_transfer_tx,
            &PairSigner::new(signer),
        )
        .await
        .map_err(|e| Error::Custom(e.to_string()))?;

    let tx_hash = tx_result.extrinsic_hash();

    let events = tx_result
        .wait_for_finalized_success()
        .await
        .map_err(|e| Error::Custom(e.to_string()))?;

    // Find a Transfer event and print it.
    let transfer_event = events
        .find_first::<RuntimeApi::balances::events::Transfer>()
        .map_err(|e| Error::Custom(e.to_string()))?;
    if let Some(event) = transfer_event {
        println!("Balance transfer success: {event:?}");
    }

    // Return the transaction hash.
    result_sender
        .send(Ok(TxResult::Substrate(tx_hash)))
        .map_err(|e| {
            Error::Custom(format!("Failed to send tx_hash: {:?}", e))
        })?;

    Ok(tx_hash)
}

async fn handle_substrate_token_tx(
    api: OnlineClient<PolkadotConfig>,
    to: AccountId32,
    amount: u128,
    asset_id: u32,
    signer: sp_core::sr25519::Pair,
    result_sender: oneshot::Sender<Result<TxResult, Error>>,
) -> Result<H256, Error> {
    let to_address = MultiAddress::Id(to.clone());
    let token_transfer_tx = RuntimeApi::tx()
        .tokens()
        .transfer(to_address, asset_id, amount);

    // Sign and submit the extrinsic.
    let tx_result = api
        .tx()
        .sign_and_submit_then_watch_default(
            &token_transfer_tx,
            &PairSigner::new(signer),
        )
        .await
        .map_err(|e| Error::Custom(e.to_string()))?;

    let tx_hash = tx_result.extrinsic_hash();

    let events = tx_result
        .wait_for_finalized_success()
        .await
        .map_err(|e| Error::Custom(e.to_string()))?;

    // Find a Transfer event and print it.
    let transfer_event = events
        .find_first::<RuntimeApi::tokens::events::Transfer>()
        .map_err(|e| Error::Custom(e.to_string()))?;
    if let Some(event) = transfer_event {
        println!("Balance transfer success: {event:?}");
    }

    // Return the transaction hash.
    result_sender
        .send(Ok(TxResult::Substrate(tx_hash)))
        .map_err(|_e| {
            Error::Custom(format!("Failed to send tx_hash: {}", tx_hash))
        })?;

    Ok(tx_hash)
}
