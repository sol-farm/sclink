//! a lightweight client for querying chainlink pricefeeds, based on commit 72a857f37516a4202431156036cb93e2b6a8d9b3
//! from https://github.com/smartcontractkit/chainlink-solana

pub mod store;

use borsh::{BorshSerialize, BorshDeserialize};
use solana_program::{self, msg, pubkey::Pubkey, entrypoint::ProgramResult, account_info::AccountInfo, program_error::ProgramError};
use static_pubkey::static_pubkey;
use store::{Transmissions, with_store};

pub const CHAINLINK_STORE_PROGRAM: Pubkey = static_pubkey!("HEvSKofvBgfaexv23kMabbYqxasxU3mQ4ibBMEmJWHny");

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


pub fn query<'info>(feed: &mut AccountInfo<'info>, scope: Scope) -> ProgramResult {
    if feed.owner.ne(&CHAINLINK_STORE_PROGRAM) {
        msg!("invalid program owner");
        return Err(ProgramError::IllegalOwner);
    }
    use std::io::Cursor;

    let mut buf = Cursor::new(Vec::with_capacity(128)); // TODO: calculate max size
    let data = match feed.data.try_borrow() {
        Ok(data) =>  data,
        Err(_) => {
            msg!("borrow failed");
            return Err(ProgramError::AccountBorrowFailed)
        }
    };
    let header = Transmissions::deserialize(&mut &data[..])?;
    match scope {
        Scope::Version => {
            let data = header.version;
            data.serialize(&mut buf)?;
        }
        Scope::Decimals => {
            let data = header.decimals;
            data.serialize(&mut buf)?;
        }
        Scope::Description => {
            // Look for the first null byte
            let end = header
                .description
                .iter()
                .position(|byte| byte == &0)
                .unwrap_or(header.description.len());

            let description = match String::from_utf8(header.description[..end].to_vec()) {
                Ok(description) => description,
                Err(err) => {
                    msg!("failed to parse description {:#?}", err);
                    return Err(ProgramError::InvalidAccountData);
                }
            };

            let data = description;
            data.serialize(&mut buf)?;
        }
        Scope::RoundData { round_id } => {
            let round = match with_store(feed, |store| store.fetch(round_id)) {
                Ok(store_info) => if let Some(info) = store_info {
                    info
                } else {
                    msg!("failed to fetch round data");
                    return Err(ProgramError::InvalidAccountData)
                },
                Err(err) => return Err(err),
            };

            let data = Round {
                round_id,
                slot: round.slot,
                answer: round.answer,
                timestamp: round.timestamp,
            };
            data.serialize(&mut buf)?;
        }
        Scope::LatestRoundData => {
            let round = match  with_store(feed, |store| store.latest()) {
                Ok(store_info) => if let Some(info) = store_info {
                    info
                } else {
                    msg!("failed to fetch round data");
                    return Err(ProgramError::InvalidAccountData)
                },
                Err(err) => return Err(err),
            };

            let data = Round {
                round_id: header.latest_round_id,
                slot: round.slot,
                answer: round.answer,
                timestamp: round.timestamp,
            };

            data.serialize(&mut buf)?;
        }
        Scope::Aggregator => {
            let data = header.writer;
            data.serialize(&mut buf)?;
        }
    }

    Ok(())
}