//! a lightweight client for querying chainlink pricefeeds, based on commit 72a857f37516a4202431156036cb93e2b6a8d9b3
//! from https://github.com/smartcontractkit/chainlink-solana

pub mod store;

use borsh::{BorshDeserialize, BorshSerialize};
use so_defi_utils::accessor::to_u32;
use so_defi_utils::accessor::AccessorType;
use solana_program::{
    self, account_info::AccountInfo, msg, program_error::ProgramError, pubkey::Pubkey,
};
use static_pubkey::static_pubkey;

use store::{with_store};
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
#[cfg_attr(not(target_arch = "bpf"), derive(Debug))]
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
    match scope {
        Scope::Version => Ok(vec![AccessorType::U8(8).access(feed)[0]]),
        Scope::Decimals => Ok(vec![AccessorType::U8(138).access(feed)[0]]),
        Scope::Description => {
            // description length is 32 bytes, so we can use the Pubkey accessor
            let description = AccessorType::Pubkey(106).access(feed);
            // Look for the first null byte
            let end = description
                .iter()
                .position(|byte| byte == &0)
                .unwrap_or(description.len());
            Ok(description[..end].to_vec())
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
                round_id: to_u32(&AccessorType::U32(143).access(feed)[..]),
                slot: round.slot,
                answer: round.answer,
                timestamp: round.timestamp,
            }
            .try_to_vec()?)
        }
        Scope::Aggregator => Ok(AccessorType::Pubkey(74).access(feed)),
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

#[cfg(test)]
mod test {
    use super::*;
    use solana_program::account_info::IntoAccountInfo;
    use static_pubkey::static_pubkey;
    #[test]
    fn test_query() {
        let rpc = solana_client::rpc_client::RpcClient::new("https://ssc-dao.genesysgo.net");
        let btc_feed = static_pubkey!("CGmWwBNsTRDENT5gmVZzRu38GnNnMm1K5C3sFiUUyYQX");
        let btc_feed_account = rpc.get_account(&btc_feed).unwrap();
        let mut btc_feed_tup = (btc_feed, btc_feed_account);
        let btc_feed_info = btc_feed_tup.into_account_info();
        let version = version(&btc_feed_info).unwrap();
        let decimals = decimals(&btc_feed_info).unwrap();
        let description = description(&btc_feed_info).unwrap();
        let latest_data = latest_round_data(&btc_feed_info).unwrap();
        let agg = aggregator(&btc_feed_info).unwrap();

        assert_eq!(version, FEED_VERSION);
        assert_eq!(decimals, 8);
        assert_eq!(
            agg,
            static_pubkey!("8xfHq5ZctheZMhntmXsayHg4GtRGvDqdz4zKcjCqJgaY")
        );
        assert_eq!(description, "BTC / USD");
        assert!(latest_data.round_id >= 2177184);
        assert!(latest_data.slot >= 141757948);
        assert!(latest_data.timestamp >= 1657926454);
    }
}
