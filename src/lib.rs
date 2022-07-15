//! a lightweight client for querying chainlink pricefeeds, based on commit 72a857f37516a4202431156036cb93e2b6a8d9b3
//! from https://github.com/smartcontractkit/chainlink-solana

pub mod store;

use borsh::{BorshDeserialize, BorshSerialize};
use solana_program::{
    self, account_info::AccountInfo, msg, program_error::ProgramError, pubkey::Pubkey,
};
use static_pubkey::static_pubkey;
use store::{with_store, Transmissions};
pub const CHAINLINK_STORE_PROGRAM: Pubkey =
    static_pubkey!("HEvSKofvBgfaexv23kMabbYqxasxU3mQ4ibBMEmJWHny");

pub const FEED_VERSION: u8 = 2;

#[derive(BorshSerialize, BorshDeserialize, Clone, Copy)]
pub enum Scope {
    Version,
    Decimals,
    Description,
    RoundData { round_id: u32 },
    LatestRoundData,
    Aggregator,
    // ProposedAggregator
    // Owner
}

#[derive(BorshSerialize, BorshDeserialize, Clone, Copy)]
pub struct Round {
    pub round_id: u32,
    pub slot: u64,
    pub timestamp: u32,
    pub answer: i128,
}

pub fn query(feed: &AccountInfo, scope: Scope) -> Result<Vec<u8>, ProgramError> {
    if feed.owner.ne(&CHAINLINK_STORE_PROGRAM) {
        msg!("invalid program owner");
        return Err(ProgramError::IllegalOwner);
    }
    let data = match feed.data.try_borrow() {
        Ok(data) => data,
        Err(_) => {
            msg!("borrow failed");
            return Err(ProgramError::AccountBorrowFailed);
        }
    };
    let header = Transmissions::deserialize(&mut &data[..])?;
    match scope {
        Scope::Version => Ok(vec![header.version]),
        Scope::Decimals => Ok(vec![header.decimals]),
        Scope::Description => {
            // Look for the first null byte
            let end = header
                .description
                .iter()
                .position(|byte| byte == &0)
                .unwrap_or(header.description.len());
            Ok(header.description[..end].to_vec())
        }
        Scope::RoundData { round_id } => {
            let round = match with_store(feed, |store| store.fetch(round_id)) {
                Ok(store_info) => {
                    if let Some(info) = store_info {
                        info
                    } else {
                        msg!("failed to fetch round data");
                        return Err(ProgramError::InvalidAccountData);
                    }
                }
                Err(err) => return Err(err),
            };

            Ok(Round {
                round_id,
                slot: round.slot,
                answer: round.answer,
                timestamp: round.timestamp,
            }
            .try_to_vec()?)
        }
        Scope::LatestRoundData => {
            let round = match with_store(feed, |store| store.latest()) {
                Ok(store_info) => {
                    if let Some(info) = store_info {
                        info
                    } else {
                        msg!("failed to fetch round data");
                        return Err(ProgramError::InvalidAccountData);
                    }
                }
                Err(err) => return Err(err),
            };

            Ok(Round {
                round_id: header.latest_round_id,
                slot: round.slot,
                answer: round.answer,
                timestamp: round.timestamp,
            }
            .try_to_vec()?)
        }
        Scope::Aggregator => Ok(header.writer.to_bytes().to_vec()),
    }
}

/// Query the feed version.
pub fn version(feed: &AccountInfo) -> Result<u8, ProgramError> {
    Ok(query(feed, Scope::Version)?[0])
}

/// Returns the amount of decimal places.
pub fn decimals(feed: &AccountInfo) -> Result<u8, ProgramError> {
    Ok(query(feed, Scope::Decimals)?[0])
}

/// Returns the feed description.
pub fn description(feed: &AccountInfo) -> Result<String, ProgramError> {
    let result = query(feed, Scope::Description)?;
    if let Ok(desc) = String::from_utf8(result) {
        Ok(desc)
    } else {
        msg!("utf8 parse failed");
        Err(ProgramError::InvalidArgument)
    }
}

/// Returns round data for the latest round.
pub fn latest_round_data(feed: &AccountInfo) -> Result<Round, ProgramError> {
    Ok(Round::deserialize(
        &mut &query(feed, Scope::LatestRoundData)?[..],
    )?)
}
/// Returns the address of the underlying OCR2 aggregator.
pub fn aggregator(feed: &AccountInfo) -> Result<Pubkey, ProgramError> {
    Ok(Pubkey::new(&query(feed, Scope::Aggregator)?[..]))
}
