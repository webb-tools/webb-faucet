use std::collections::HashMap;
use std::sync::Arc;

use ethers::types::U256;
use ethers::{
    prelude::*,
    providers::{Http, Provider},
    types::{Address, TransactionReceipt},
};
use rocket::tokio::sync::oneshot;
use serde::{Deserialize, Serialize};
use webb::evm::ethers;
use webb::substrate::subxt::{
    utils::AccountId32, OnlineClient, PolkadotConfig,
};

use crate::error::Error;

pub type EthersClient = Arc<
    NonceManagerMiddleware<
        SignerMiddleware<
            gas_oracle::GasOracleMiddleware<
                gas_escalator::GasEscalatorMiddleware<Provider<Http>>,
                gas_oracle::GasNow,
            >,
            LocalWallet,
        >,
    >,
>;

pub struct EvmProviders<T> {
    pub providers: HashMap<u64, T>,
}

pub struct SubstrateProviders<T> {
    pub providers: HashMap<u64, T>,
}

#[derive(Debug, Serialize, Deserialize)]
pub enum TxResult {
    Evm(TransactionReceipt),
    Substrate(webb::substrate::subxt::utils::H256),
}

#[allow(clippy::large_enum_variant)]
pub enum Transaction {
    Evm {
        provider: EthersClient,
        to: Address,
        amount: U256,
        token_address: Option<Address>,
        result_sender: oneshot::Sender<Result<TxResult, Error>>,
    },
    Substrate {
        api: OnlineClient<PolkadotConfig>,
        to: AccountId32,
        amount: u128,
        asset_id: Option<u32>,
        signer: sp_core::sr25519::Pair,
        result_sender: oneshot::Sender<Result<TxResult, Error>>,
    },
}

impl std::fmt::Debug for Transaction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Evm {
                provider,
                to,
                amount,
                token_address,
                result_sender,
            } => f
                .debug_struct("Evm")
                .field("provider", provider)
                .field("to", to)
                .field("amount", amount)
                .field("token_address", token_address)
                .field("result_sender", result_sender)
                .finish(),
            Self::Substrate {
                api,
                to,
                amount,
                asset_id,
                result_sender,
                ..
            } => f
                .debug_struct("Substrate")
                .field("api", api)
                .field("to", to)
                .field("amount", amount)
                .field("asset_id", asset_id)
                .field("signer", &"<hidden>")
                .field("result_sender", result_sender)
                .finish(),
        }
    }
}
