//! store account types, values, etc.. extracted from https://github.com/smartcontractkit/chainlink-solana/blob/develop/contracts/programs/store/src/lib.rs
use crate::FEED_VERSION;
use borsh::{BorshDeserialize, BorshSerialize};
use so_defi_utils::accessor::{to_u32, AccessorType};
use solana_program::account_info::AccountInfo;
use solana_program::msg;
use solana_program::program_error::ProgramError;
use solana_program::pubkey::Pubkey;

pub const HEADER_SIZE: usize = 192;

#[repr(C)]
pub struct Store {
    pub __discriminator: [u8; 8],
    pub owner: Pubkey,
    pub proposed_owner: Pubkey,
    pub lowering_access_controller: Pubkey,
}

#[repr(C)]
#[derive(BorshSerialize, BorshDeserialize)]
pub struct NewTransmission {
    pub timestamp: u64,
    pub answer: i128,
}

#[repr(C)]
#[derive(
    Debug, Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, bytemuck::Pod, bytemuck::Zeroable,
)]
pub struct Transmission {
    pub slot: u64,
    pub timestamp: u32,
    pub _padding0: u32,
    pub answer: i128,
    pub _padding1: u64,
    pub _padding2: u64,
}

use std::cell::Ref;

use std::mem::size_of;

#[cfg_attr(not(target_arch = "bpf"), derive(Debug))]
/// Two ringbuffers
/// - Live one that has a day's worth of data that's updated every second
/// - Historical one that stores historical data
pub struct Feed<'a> {
    pub header: &'a mut Box<Transmissions>,
    live: &'a mut [Transmission],
    historical: &'a mut [Transmission],
}

#[derive(BorshSerialize, BorshDeserialize, Clone)]
// note about the type layout: it incorrectly detects a padding of 20 bytes at the start
// and 151 bytes in the middle, subtract those values from the offsets
#[cfg_attr(
    not(target_arch = "bpf"),
    derive(Debug),
    derive(type_layout::TypeLayout)
)]
pub struct Transmissions {
    pub _discriminator: [u8; 8],
    pub version: u8,
    pub state: u8,
    pub owner: Pubkey,
    pub proposed_owner: Pubkey,
    pub writer: Pubkey,
    /// Raw UTF-8 byte string
    pub description: [u8; 32],
    pub decimals: u8,
    pub flagging_threshold: u32,
    pub latest_round_id: u32,
    pub granularity: u8,
    pub live_length: u32,
    live_cursor: u32,
    historical_cursor: u32,
}

impl Transmissions {
    pub const NORMAL: u8 = 0;
    pub const FLAGGED: u8 = 1;
}

pub fn with_store<'a, 'info: 'a, F, T>(
    account: &AccountInfo<'info>,
    f: F,
) -> Result<T, ProgramError>
where
    F: FnOnce(&mut Feed) -> T,
{
    let n = {
        let response = AccessorType::U8(8).access(account);
        if response[0].ne(&FEED_VERSION) {
            msg!("invalid feed version");
            return Err(ProgramError::InvalidAccountData);
        }
        to_u32(&AccessorType::U32(148).access(account)[..]) as usize
    };

    let (live, historical) = {
        let data = account.try_borrow_data()?;
        let (live, hist) = Ref::map_split(data, |data| {
            // skip the header
            let (_header, data) = data.split_at(8 + HEADER_SIZE); // discriminator + header size
            let (live, historical) = data.split_at(n * size_of::<Transmission>());
            // NOTE: no try_map_split available..
            let live = bytemuck::try_cast_slice::<_, Transmission>(live).unwrap();
            let historical = bytemuck::try_cast_slice::<_, Transmission>(historical).unwrap();
            (live, historical)
        });
        (live, hist)
    };
    let mut live = live.to_vec();
    let mut historical = historical.to_vec();
    let data = account.try_borrow_data()?;
    let transmission = Transmissions::deserialize(&mut &data[..]).unwrap();
    let mut store = Feed {
        header: &mut Box::new(transmission),
        live: &mut live[..],
        historical: &mut historical[..],
    };
    Ok(f(&mut store))
}

impl<'a> Feed<'a> {
    pub fn insert(&mut self, round: Transmission) {
        self.header.latest_round_id += 1;

        // insert into live data
        self.live[self.header.live_cursor as usize] = round;
        self.header.live_cursor = (self.header.live_cursor + 1) % self.live.len() as u32;

        if self.header.latest_round_id % self.header.granularity as u32 == 0 {
            // insert into historical data
            self.historical[self.header.historical_cursor as usize] = round;
            self.header.historical_cursor =
                (self.header.historical_cursor + 1) % self.historical.len() as u32;
        }
    }

    pub fn latest(&self) -> Option<Transmission> {
        if self.header.latest_round_id == 0 {
            return None;
        }

        let len = self.header.live_length;
        // Handle wraparound
        let i = (self.header.live_cursor + len.saturating_sub(1)) % len;

        Some(self.live[i as usize])
    }

    pub fn fetch(&self, round_id: u32) -> Option<Transmission> {
        if self.header.latest_round_id < round_id {
            return None;
        }

        let latest_round_id = self.header.latest_round_id;
        let granularity = self.header.granularity as u32;

        // if in live range, fetch from live set
        let live_start = latest_round_id.saturating_sub((self.live.len() as u32).saturating_sub(1));
        // if in historical range, fetch from closest
        let historical_end = latest_round_id - (latest_round_id % granularity);
        let historical_start = historical_end
            .saturating_sub(granularity * (self.historical.len() as u32).saturating_sub(1));

        if (live_start..=latest_round_id).contains(&round_id) {
            // live data
            let offset = latest_round_id - round_id;
            let offset = offset + 1; // + 1 because we're looking for the element before the cursor

            let index = self
                .header
                .live_cursor
                .checked_sub(offset)
                .unwrap_or(self.live.len() as u32 - (offset - self.header.live_cursor));

            Some(self.live[index as usize])
        } else if (historical_start..=historical_end).contains(&round_id) {
            // historical data
            let round_id = round_id - (round_id % granularity);
            let offset = (historical_end - round_id) / granularity;
            let offset = offset + 1; // + 1 because we're looking for the element before the cursor

            let index = self
                .header
                .historical_cursor
                .checked_sub(offset)
                .unwrap_or({
                    self.historical.len() as u32 - (offset - self.header.historical_cursor)
                });

            Some(self.historical[index as usize])
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use solana_program::account_info::IntoAccountInfo;
    use static_pubkey::static_pubkey;
    use type_layout::TypeLayout;

    use super::*;
    #[test]
    fn transmission_ffset() {
        println!("{}", Transmissions::type_layout());
    }
    #[test]
    fn transmissions_btc() {
        let rpc = solana_client::rpc_client::RpcClient::new("https://ssc-dao.genesysgo.net");
        let btc_feed = static_pubkey!("CGmWwBNsTRDENT5gmVZzRu38GnNnMm1K5C3sFiUUyYQX");
        let btc_feed_account = rpc.get_account(&btc_feed).unwrap();
        let mut btc_feed_tup = (btc_feed, btc_feed_account);
        let btc_feed_info = btc_feed_tup.into_account_info();
        with_store(&btc_feed_info, |feed| {
            assert_eq!(feed.header.live_length, 86400);
        })
        .unwrap();
        let latest_round_id = to_u32(&AccessorType::U32(143).access(&btc_feed_info)[..]);
        // latest round as of jul 15th
        assert!(latest_round_id >= 2176986);
    }
    #[test]
    fn transmissions() {
        let live_length = 2;
        let historical_length = 3;
        let mut data = vec![
            0;
            8 + HEADER_SIZE
                + (live_length + historical_length) * size_of::<Transmission>()
        ];
        let header = &mut data[..8 + HEADER_SIZE]; // use a subslice to ensure the header fits into HEADER_SIZE bytes
        let mut cursor = std::io::Cursor::new(header);

        // insert the initial header with some granularity
        Transmissions {
            _discriminator: [0_u8; 8],
            version: 2,
            state: Transmissions::NORMAL,
            owner: Pubkey::default(),
            proposed_owner: Pubkey::default(),
            writer: Pubkey::default(),
            description: [0; 32],
            decimals: 18,
            flagging_threshold: 1000,
            latest_round_id: 0,
            granularity: 5,
            live_length: live_length as u32,
            live_cursor: 0,
            historical_cursor: 0,
        }
        .serialize(&mut cursor)
        .unwrap();

        let mut lamports = 0u64;

        let pubkey = Pubkey::default();
        let info = AccountInfo::new(
            &pubkey,
            false,
            false,
            &mut lamports,
            &mut data,
            &crate::CHAINLINK_STORE_PROGRAM,
            false,
            0,
        );

        with_store(&info, |store| {
            for i in 1..=20 {
                store.insert(Transmission {
                    slot: u64::from(i),
                    answer: i128::from(i),
                    timestamp: i,
                    ..Default::default()
                });
            }

            assert_eq!(store.fetch(21), None);
            // Live range returns precise round
            assert_eq!(
                store.fetch(20),
                Some(Transmission {
                    slot: 20,
                    answer: 20,
                    timestamp: 20,
                    ..Default::default()
                })
            );
            assert_eq!(
                store.fetch(19),
                Some(Transmission {
                    slot: 19,
                    answer: 19,
                    timestamp: 19,
                    ..Default::default()
                })
            );
            // Historical range rounds down
            assert_eq!(
                store.fetch(18),
                Some(Transmission {
                    slot: 15,
                    answer: 15,
                    timestamp: 15,
                    ..Default::default()
                })
            );
            assert_eq!(
                store.fetch(15),
                Some(Transmission {
                    slot: 15,
                    answer: 15,
                    timestamp: 15,
                    ..Default::default()
                })
            );
            assert_eq!(
                store.fetch(14),
                Some(Transmission {
                    slot: 10,
                    answer: 10,
                    timestamp: 10,
                    ..Default::default()
                })
            );
            assert_eq!(
                store.fetch(10),
                Some(Transmission {
                    slot: 10,
                    answer: 10,
                    timestamp: 10,
                    ..Default::default()
                })
            );
            // Out of range
            assert_eq!(store.fetch(9), None);
        })
        .unwrap();
    }
}
