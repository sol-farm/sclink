//! a lightweight client for querying chainlink pricefeeds, based on commit 72a857f37516a4202431156036cb93e2b6a8d9b3
//! from https://github.com/smartcontractkit/chainlink-solana

pub mod store;

use std::cell::Ref;
use std::mem::size_of;

use borsh::{BorshDeserialize, BorshSerialize};
use so_defi_utils::accessor::to_u32;
use so_defi_utils::accessor::AccessorType;
use solana_program::{
    self, account_info::AccountInfo, msg, program_error::ProgramError, pubkey::Pubkey,
};
use static_pubkey::static_pubkey;

use store::with_store;

use crate::store::HEADER_SIZE;
use crate::store::Transmission;
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
    LatestRoundDataWithDecimals,
    LatestRoundDataWithDecimals2,
    // ProposedAggregator
    // Owner
}

#[derive(BorshSerialize, BorshDeserialize, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(not(target_arch = "bpf"), derive(Debug))]
pub struct Round {
    pub round_id: u32,
    pub slot: u64,
    pub timestamp: u32,
    pub answer: i128,
}

#[derive(BorshSerialize, BorshDeserialize, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(not(target_arch = "bpf"), derive(Debug))]
pub struct RoundWithDecimals {
    pub round: Round,
    pub decimals: u8,
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
        Scope::LatestRoundDataWithDecimals => {
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
            Ok(RoundWithDecimals {
                round: Round {
                    round_id: to_u32(&AccessorType::U32(143).access(feed)[..]),
                    slot: round.slot,
                    answer: round.answer,
                    timestamp: round.timestamp,
                },
                decimals: AccessorType::U8(138).access(feed)[0],
            }
            .try_to_vec()?)
        }
        Scope::LatestRoundDataWithDecimals2 => {
                msg!("checking feed version");
                let response = AccessorType::U8(8).access(feed);
                if response[0].ne(&FEED_VERSION) {
                    msg!("invalid feed version");
                    return Err(ProgramError::InvalidAccountData);
                }
                
                let n = to_u32(&AccessorType::U32(148).access(feed)[..]) as usize;
                {
                    
                    let (live, _) = {
                        let data = feed.try_borrow_data()?;
                        std::cell::Ref::map_split(data, |data| {
                            // skip the header
                            let (_header, data) = data.split_at(8 + HEADER_SIZE); // discriminator + header size
                            let (live, historical) = data.split_at(n * size_of::<Transmission>());
                            // NOTE: no try_map_split available..
                            let live = bytemuck::try_cast_slice::<_, Transmission>(live).unwrap();
                            let historical = bytemuck::try_cast_slice::<_, Transmission>(historical).unwrap();
                            (live, historical)
                        })
                    };
                    let transmission = crate::store::Transmissions::deserialize(&mut &feed.try_borrow_data()?[..]).unwrap();
                    if transmission.latest_round_id == 0 {
                        panic!("found is none");
                    }
                    let len = transmission.live_length;
                    let idx = (transmission.live_cursor + len.saturating_sub(1)) % len;
                    let (
                        slot,
                        answer,
                        timestamp
                    ) = {
                        let round_data = &live[idx as usize];
                        (round_data.slot, round_data.answer, round_data.timestamp)
                    };
                    Ok(RoundWithDecimals {
                        round: Round {
                            round_id: to_u32(&AccessorType::U32(143).access(feed)[..]),
                            slot: slot,
                            answer: answer,
                            timestamp: timestamp,
                        },
                        decimals: AccessorType::U8(138).access(feed)[0],
                    }.try_to_vec()?)
                }
        }
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

/// Returns round data for the latest round, including decimal value
pub fn latest_round_data_with_decimals(
    feed: &AccountInfo,
) -> Result<RoundWithDecimals, ProgramError> {
    Ok(RoundWithDecimals::deserialize(
        &mut &query(feed, Scope::LatestRoundDataWithDecimals)?[..],
    )?)
}

/// same as latest_round_data_with_decimals2 but attempts to reduce the number of allocations
pub fn latest_round_data_with_decimals2(
    feed: &AccountInfo,
) -> Result<RoundWithDecimals, ProgramError> {
    Ok(RoundWithDecimals::deserialize(
        &mut &query(feed, Scope::LatestRoundDataWithDecimals2)?[..],
    )?)
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
        println!("{}", latest_data.answer);

        let latest_with_dec = latest_round_data_with_decimals(&btc_feed_info).unwrap();
        assert_eq!(latest_data, latest_with_dec.round);
        assert_eq!(latest_with_dec.decimals, 8);

        let latest_with_dec2 = latest_round_data_with_decimals2(&btc_feed_info).unwrap();
        assert_eq!(latest_data, latest_with_dec2.round);
        assert_eq!(latest_with_dec2.decimals, 8);
    }
}
