use std::{
    borrow::Cow,
    io::Cursor,
    ops::{Deref, DerefMut},
};

use crate::asset::{Asset, Balance};
use crate::crypto::{KeyPair, PublicKey, ScriptHash, SigPair};
use crate::script::Script;
use crate::serializer::*;

#[macro_use]
mod util;

pub mod tx_type;
pub use self::tx_type::*;

pub trait EncodeTx {
    fn encode(&self, v: &mut Vec<u8>);
}

pub trait DecodeTx<T> {
    fn decode(cur: &mut Cursor<&[u8]>, tx: Tx) -> Option<T>;
}

pub trait SignTx {
    fn sign(&self, key_pair: &KeyPair) -> SigPair;
    fn append_sign(&mut self, key_pair: &KeyPair);
}

#[derive(Clone, Debug)]
pub enum TxVariant {
    OwnerTx(OwnerTx),
    MintTx(MintTx),
    RewardTx(RewardTx),
    TransferTx(TransferTx),
}

impl TxVariant {
    pub fn encode(&self, v: &mut Vec<u8>) {
        match self {
            TxVariant::OwnerTx(tx) => tx.encode(v),
            TxVariant::MintTx(tx) => tx.encode(v),
            TxVariant::RewardTx(tx) => tx.encode(v),
            TxVariant::TransferTx(tx) => tx.encode(v),
        };
    }

    pub fn encode_with_sigs(&self, v: &mut Vec<u8>) {
        macro_rules! encode_sigs {
            ($name:expr, $vec:expr) => {{
                $vec.push_u16($name.signature_pairs.len() as u16);
                for sig in &$name.signature_pairs {
                    $vec.push_sig_pair(sig)
                }
                $name.encode($vec);
            }};
        }

        match self {
            TxVariant::OwnerTx(tx) => encode_sigs!(tx, v),
            TxVariant::MintTx(tx) => encode_sigs!(tx, v),
            TxVariant::RewardTx(tx) => encode_sigs!(tx, v),
            TxVariant::TransferTx(tx) => encode_sigs!(tx, v),
        };
    }

    pub fn decode_with_sigs(cur: &mut Cursor<&[u8]>) -> Option<TxVariant> {
        let sigs = {
            let len = cur.take_u16().ok()?;
            let mut vec = Vec::with_capacity(len as usize);
            for _ in 0..len {
                vec.push(cur.take_sig_pair().ok()?)
            }
            vec
        };
        let mut base = Tx::decode_base(cur)?;
        base.signature_pairs = sigs;
        match base.tx_type {
            TxType::OWNER => Some(TxVariant::OwnerTx(OwnerTx::decode(cur, base)?)),
            TxType::MINT => Some(TxVariant::MintTx(MintTx::decode(cur, base)?)),
            TxType::REWARD => Some(TxVariant::RewardTx(RewardTx::decode(cur, base)?)),
            TxType::TRANSFER => Some(TxVariant::TransferTx(TransferTx::decode(cur, base)?)),
        }
    }
}

impl Deref for TxVariant {
    type Target = Tx;

    fn deref(&self) -> &Self::Target {
        match self {
            TxVariant::OwnerTx(tx) => &tx.base,
            TxVariant::MintTx(tx) => &tx.base,
            TxVariant::RewardTx(tx) => &tx.base,
            TxVariant::TransferTx(tx) => &tx.base,
        }
    }
}

impl DerefMut for TxVariant {
    fn deref_mut(&mut self) -> &mut Tx {
        match self {
            TxVariant::OwnerTx(tx) => &mut tx.base,
            TxVariant::MintTx(tx) => &mut tx.base,
            TxVariant::RewardTx(tx) => &mut tx.base,
            TxVariant::TransferTx(tx) => &mut tx.base,
        }
    }
}

impl<'a> Into<Cow<'a, TxVariant>> for TxVariant {
    fn into(self) -> Cow<'a, TxVariant> {
        Cow::Owned(self)
    }
}

impl<'a> Into<Cow<'a, TxVariant>> for &'a TxVariant {
    fn into(self) -> Cow<'a, TxVariant> {
        Cow::Borrowed(self)
    }
}

#[derive(Clone, Debug)]
pub struct Tx {
    pub tx_type: TxType,
    pub timestamp: u64,
    pub fee: Asset,
    pub signature_pairs: Vec<SigPair>,
}

impl Tx {
    fn encode_base(&self, v: &mut Vec<u8>) {
        v.push(self.tx_type as u8);
        v.push_u64(self.timestamp);
        v.push_asset(&self.fee);
    }

    fn decode_base(cur: &mut Cursor<&[u8]>) -> Option<Tx> {
        let tx_type = match cur.take_u8().ok()? {
            t if t == TxType::OWNER as u8 => TxType::OWNER,
            t if t == TxType::MINT as u8 => TxType::MINT,
            t if t == TxType::REWARD as u8 => TxType::REWARD,
            t if t == TxType::TRANSFER as u8 => TxType::TRANSFER,
            _ => return None,
        };
        let timestamp = cur.take_u64().ok()?;
        let fee = cur.take_asset().ok()?;

        Some(Tx {
            tx_type,
            timestamp,
            fee,
            signature_pairs: Vec::new(),
        })
    }
}

impl PartialEq for Tx {
    fn eq(&self, other: &Self) -> bool {
        self.tx_type == other.tx_type
            && self.timestamp == other.timestamp
            && self.fee.eq(&other.fee).unwrap_or(false)
            && self.signature_pairs == other.signature_pairs
    }
}

#[derive(Clone, Debug)]
pub struct OwnerTx {
    pub base: Tx,
    pub minter: PublicKey,  // Key that signs blocks
    pub wallet: ScriptHash, // Hot wallet that receives rewards
    pub script: Script,     // Hot wallet previous script
}

impl EncodeTx for OwnerTx {
    fn encode(&self, v: &mut Vec<u8>) {
        self.encode_base(v);
        v.push_pub_key(&self.minter);
        v.push_script_hash(&self.wallet);
        v.push_bytes(&self.script);
    }
}

impl DecodeTx<OwnerTx> for OwnerTx {
    fn decode(cur: &mut Cursor<&[u8]>, tx: Tx) -> Option<OwnerTx> {
        assert_eq!(tx.tx_type, TxType::OWNER);
        let minter = cur.take_pub_key().ok()?;
        let wallet = cur.take_script_hash().ok()?;
        let script = cur.take_bytes().ok()?.into();
        Some(OwnerTx {
            base: tx,
            minter,
            wallet,
            script,
        })
    }
}

#[derive(Clone, Debug)]
pub struct MintTx {
    pub base: Tx,
    pub to: ScriptHash,
    pub balance: Balance,
    pub script: Script,
}

impl EncodeTx for MintTx {
    fn encode(&self, v: &mut Vec<u8>) {
        self.encode_base(v);
        v.push_script_hash(&self.to);
        v.push_balance(&self.balance);
        v.push_bytes(&self.script);
    }
}

impl DecodeTx<MintTx> for MintTx {
    fn decode(cur: &mut Cursor<&[u8]>, tx: Tx) -> Option<Self> {
        assert_eq!(tx.tx_type, TxType::MINT);
        let to = cur.take_script_hash().ok()?;
        let balance = cur.take_balance().ok()?;
        let script = Script::from(cur.take_bytes().ok()?);
        Some(Self {
            base: tx,
            to,
            balance,
            script,
        })
    }
}

#[derive(Clone, Debug)]
pub struct RewardTx {
    pub base: Tx,
    pub to: ScriptHash,
    pub rewards: Balance,
}

impl EncodeTx for RewardTx {
    fn encode(&self, v: &mut Vec<u8>) {
        debug_assert_eq!(self.base.signature_pairs.len(), 0);
        self.encode_base(v);
        v.push_script_hash(&self.to);
        v.push_balance(&self.rewards);
    }
}

impl DecodeTx<RewardTx> for RewardTx {
    fn decode(cur: &mut Cursor<&[u8]>, tx: Tx) -> Option<RewardTx> {
        assert_eq!(tx.tx_type, TxType::REWARD);
        let key = cur.take_script_hash().ok()?;
        let rewards = cur.take_balance().ok()?;

        Some(RewardTx {
            base: tx,
            to: key,
            rewards,
        })
    }
}

#[derive(Clone, Debug)]
pub struct TransferTx {
    pub base: Tx,
    pub from: ScriptHash,
    pub to: ScriptHash,
    pub script: Script,
    pub amount: Asset,
    pub memo: Vec<u8>,
}

impl EncodeTx for TransferTx {
    fn encode(&self, v: &mut Vec<u8>) {
        self.encode_base(v);
        v.push_script_hash(&self.from);
        v.push_script_hash(&self.to);
        v.push_bytes(&self.script);
        v.push_asset(&self.amount);
        v.push_bytes(&self.memo);
    }
}

impl DecodeTx<TransferTx> for TransferTx {
    fn decode(cur: &mut Cursor<&[u8]>, tx: Tx) -> Option<TransferTx> {
        assert_eq!(tx.tx_type, TxType::TRANSFER);
        let from = cur.take_script_hash().ok()?;
        let to = cur.take_script_hash().ok()?;
        let script = cur.take_bytes().ok()?.into();
        let amount = cur.take_asset().ok()?;
        let memo = cur.take_bytes().ok()?;
        Some(TransferTx {
            base: tx,
            from,
            to,
            script,
            amount,
            memo,
        })
    }
}

tx_deref!(OwnerTx);
tx_deref!(MintTx);
tx_deref!(RewardTx);
tx_deref!(TransferTx);

tx_sign!(OwnerTx);
tx_sign!(MintTx);
tx_sign!(TransferTx);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto;

    macro_rules! cmp_base_tx {
        ($id:ident, $ty:expr, $ts:expr, $fee:expr) => {
            assert_eq!($id.tx_type, $ty);
            assert_eq!($id.timestamp, $ts);
            assert_eq!($id.fee.to_string(), $fee);
        };
    }

    #[test]
    fn test_encode_tx_with_sigs() {
        let to = crypto::KeyPair::gen_keypair();
        let reward_tx = TxVariant::RewardTx(RewardTx {
            base: Tx {
                tx_type: TxType::REWARD,
                timestamp: 123,
                fee: get_asset("123 GOLD"),
                signature_pairs: vec![],
            },
            to: to.0.into(),
            rewards: Balance::from(get_asset("1.50 GOLD"), get_asset("1.0 SILVER")).unwrap(),
        });

        let mut v = vec![];
        reward_tx.encode_with_sigs(&mut v);

        let mut c = Cursor::<&[u8]>::new(&v);
        TxVariant::decode_with_sigs(&mut c).unwrap();
    }

    #[test]
    fn test_encode_owner() {
        let minter = crypto::KeyPair::gen_keypair();
        let wallet = crypto::KeyPair::gen_keypair();
        let owner_tx = OwnerTx {
            base: Tx {
                tx_type: TxType::OWNER,
                timestamp: 1230,
                fee: get_asset("123 GOLD"),
                signature_pairs: vec![],
            },
            minter: minter.0,
            wallet: wallet.0.clone().into(),
            script: wallet.0.into(),
        };

        let mut v = vec![];
        owner_tx.encode(&mut v);

        let mut c = Cursor::<&[u8]>::new(&v);
        let base = Tx::decode_base(&mut c).unwrap();
        let dec = OwnerTx::decode(&mut c, base).unwrap();

        cmp_base_tx!(dec, TxType::OWNER, 1230, "123 GOLD");
        assert_eq!(owner_tx.minter, dec.minter);
        assert_eq!(owner_tx.wallet, dec.wallet);
    }

    #[test]
    fn test_encode_mint() {
        let wallet = crypto::KeyPair::gen_keypair();
        let mint_tx = MintTx {
            base: Tx {
                tx_type: TxType::MINT,
                timestamp: 1234,
                fee: get_asset("123 GOLD"),
                signature_pairs: vec![],
            },
            to: wallet.0.clone().into(),
            balance: Balance::from("10 GOLD".parse().unwrap(), "100 SILVER".parse().unwrap())
                .unwrap(),
            script: wallet.0.into(),
        };

        let mut v = vec![];
        mint_tx.encode(&mut v);

        let mut c = Cursor::<&[u8]>::new(&v);
        let base = Tx::decode_base(&mut c).unwrap();
        let dec = MintTx::decode(&mut c, base).unwrap();

        cmp_base_tx!(dec, TxType::MINT, 1234, "123 GOLD");
        assert_eq!(mint_tx.to, dec.to);
        assert_eq!(
            mint_tx.balance.gold().to_string(),
            dec.balance.gold().to_string()
        );
        assert_eq!(
            mint_tx.balance.silver().to_string(),
            dec.balance.silver().to_string()
        );
    }

    #[test]
    fn test_encode_reward() {
        let to = crypto::KeyPair::gen_keypair();
        let reward_tx = RewardTx {
            base: Tx {
                tx_type: TxType::REWARD,
                timestamp: 123,
                fee: get_asset("123 GOLD"),
                signature_pairs: vec![],
            },
            to: to.0.into(),
            rewards: Balance::from(get_asset("1.50 GOLD"), get_asset("1.0 SILVER")).unwrap(),
        };

        let mut v = vec![];
        reward_tx.encode(&mut v);

        let mut c = Cursor::<&[u8]>::new(&v);
        let base = Tx::decode_base(&mut c).unwrap();
        let dec = RewardTx::decode(&mut c, base).unwrap();

        cmp_base_tx!(dec, TxType::REWARD, 123, "123 GOLD");
        assert_eq!(reward_tx.to, dec.to);
        assert_eq!(reward_tx.rewards, dec.rewards);
    }

    #[test]
    fn test_encode_transfer() {
        let from = crypto::KeyPair::gen_keypair();
        let to = crypto::KeyPair::gen_keypair();
        let transfer_tx = TransferTx {
            base: Tx {
                tx_type: TxType::TRANSFER,
                timestamp: 1234567890,
                fee: get_asset("1.23 GOLD"),
                signature_pairs: vec![],
            },
            from: from.0.into(),
            to: to.0.into(),
            script: vec![1, 2, 3, 4].into(),
            amount: get_asset("1.0456 GOLD"),
            memo: Vec::from(String::from("Hello world!").as_bytes()),
        };

        let mut v = vec![];
        transfer_tx.encode(&mut v);

        let mut c = Cursor::<&[u8]>::new(&v);
        let base = Tx::decode_base(&mut c).unwrap();
        let dec = TransferTx::decode(&mut c, base).unwrap();

        cmp_base_tx!(dec, TxType::TRANSFER, 1234567890, "1.23 GOLD");
        assert_eq!(transfer_tx.from, dec.from);
        assert_eq!(transfer_tx.to, dec.to);
        assert_eq!(transfer_tx.script, vec![1, 2, 3, 4].into());
        assert_eq!(transfer_tx.amount.to_string(), dec.amount.to_string());
        assert_eq!(transfer_tx.memo, dec.memo);
    }

    #[test]
    fn test_tx_eq() {
        let tx_a = Tx {
            tx_type: TxType::MINT,
            timestamp: 1000,
            fee: get_asset("10 GOLD"),
            signature_pairs: vec![KeyPair::gen_keypair().sign(b"hello world")],
        };
        let tx_b = tx_a.clone();
        assert_eq!(tx_a, tx_b);

        let mut tx_b = tx_a.clone();
        tx_b.fee = get_asset("10.0 GOLD");
        assert_eq!(tx_a, tx_b);

        let mut tx_b = tx_a.clone();
        tx_b.tx_type = TxType::OWNER;
        assert_ne!(tx_a, tx_b);

        let mut tx_b = tx_a.clone();
        tx_b.timestamp = tx_b.timestamp + 1;
        assert_ne!(tx_a, tx_b);

        let mut tx_b = tx_a.clone();
        tx_b.fee = get_asset("10 SILVER");
        assert_ne!(tx_a, tx_b);

        let mut tx_b = tx_a.clone();
        tx_b.fee = get_asset("1.0 GOLD");
        assert_ne!(tx_a, tx_b);

        let mut tx_b = tx_a.clone();
        tx_b.signature_pairs
            .push(KeyPair::gen_keypair().sign(b"hello world"));
        assert_ne!(tx_a, tx_b);
    }

    fn get_asset(s: &str) -> Asset {
        s.parse().unwrap()
    }
}
