use sodiumoxide::crypto::sign;
use sodiumoxide::randombytes;
use bs58;

use super::sigpair::*;
use super::error::*;
use super::util::*;

pub const PUB_ADDRESS_PREFIX: &str = "GOD";
const PRIV_BUF_PREFIX: u8 = 0x01;
const PUB_BUF_PREFIX: u8 = 0x02;

pub trait Wif<T> {
    fn from_wif(s: &str) -> Result<T, WifError>;
    fn to_wif(&self) -> Box<str>;
}

#[derive(Debug, Clone, PartialEq)]
pub struct PublicKey(pub(crate) sign::PublicKey);

impl PublicKey {
    #[inline]
    pub fn verify(&self, msg: &[u8], sig: &sign::Signature) -> bool {
        sign::verify_detached(sig, msg, &self.0)
    }

    #[inline]
    pub fn from_slice(bytes: &[u8]) -> Option<PublicKey> {
        let key = sign::PublicKey::from_slice(bytes)?;
        Some(PublicKey(key))
    }
}

impl Wif<PublicKey> for PublicKey {
    fn from_wif(s: &str) -> Result<PublicKey, WifError> {
        if s.len() < 3 || &s[0..3] != PUB_ADDRESS_PREFIX {
            return Err(WifError::new(WifErrorKind::InvalidPrefix))
        }
        let raw = match bs58::decode(&s[3..]).into_vec() {
            Ok(bytes) => bytes,
            Err(_) => return Err(WifError::new(WifErrorKind::InvalidBs58Encoding))
        };
        if raw.len() != 37 {
            return Err(WifError::new(WifErrorKind::InvalidLen))
        } else if raw[0] != PUB_BUF_PREFIX {
            return Err(WifError::new(WifErrorKind::InvalidPrefix))
        }

        let prefixed_key = &raw[0..raw.len() - 4];
        {
            let checksum_a = &raw[raw.len() - 4 .. raw.len()];
            let checksum_b = &double_sha256(prefixed_key)[0..4];
            if checksum_a != checksum_b {
                return Err(WifError::new(WifErrorKind::InvalidChecksum))
            }
        }

        let key = &prefixed_key[1 .. prefixed_key.len()];
        Ok(PublicKey(sign::PublicKey::from_slice(key).unwrap()))
    }

    fn to_wif(&self) -> Box<str> {
        let mut buf: Vec<u8> = Vec::<u8>::with_capacity(37);
        buf.push(PUB_BUF_PREFIX);
        buf.extend_from_slice(self.0.as_ref());

        let checksum = &double_sha256(&buf)[0..4];
        buf.extend_from_slice(checksum);

        let mut s = bs58::encode(buf).into_string();
        s.insert_str(0, PUB_ADDRESS_PREFIX);
        s.into_boxed_str()
    }
}

impl AsRef<[u8]> for PublicKey {
    fn as_ref(&self) -> &[u8] {
        self.0.as_ref()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct PrivateKey {
    seed: sign::Seed,
    key: sign::SecretKey
}

impl PrivateKey {
    #[inline]
    pub fn sign(&self, msg: &[u8]) -> sign::Signature {
        sign::sign_detached(msg, &self.key)
    }

    #[inline]
    pub fn from_slice(seed: &[u8], key: &[u8]) -> Option<PrivateKey> {
        Some(PrivateKey {
            seed: sign::Seed::from_slice(seed)?,
            key: sign::SecretKey::from_slice(key)?
        })
    }
}

impl Wif<KeyPair> for PrivateKey {
    fn from_wif(s: &str) -> Result<KeyPair, WifError> {
        let raw = match bs58::decode(s).into_vec() {
            Ok(bytes) => bytes,
            Err(_) => return Err(WifError::new(WifErrorKind::InvalidBs58Encoding))
        };
        if raw.len() != 37 {
            return Err(WifError::new(WifErrorKind::InvalidLen))
        } else if raw[0] != PRIV_BUF_PREFIX {
            return Err(WifError::new(WifErrorKind::InvalidPrefix))
        }

        let key = &raw[0..raw.len() - 4];
        {
            let checksum_a = &raw[raw.len() - 4 .. raw.len()];
            let checksum_b = &double_sha256(key)[0..4];
            if checksum_a != checksum_b {
                return Err(WifError::new(WifErrorKind::InvalidChecksum))
            }
        }

        let seed = sign::Seed::from_slice(&key[1..]).unwrap();
        let (pk, sk) = sign::keypair_from_seed(&seed);
        Ok(KeyPair(PublicKey(pk), PrivateKey {
            seed,
            key: sk
        }))
    }

    fn to_wif(&self) -> Box<str> {
        let mut buf = Vec::<u8>::with_capacity(37);
        buf.push(PRIV_BUF_PREFIX);
        buf.extend_from_slice(&self.seed.0);

        let checksum = &double_sha256(&buf)[0..4];
        buf.extend_from_slice(checksum);

        bs58::encode(buf).into_string().into_boxed_str()
    }
}

#[derive(Debug, Clone)]
pub struct KeyPair(pub PublicKey, pub PrivateKey);

impl KeyPair {
    #[inline]
    pub fn sign(&self, msg: &[u8]) -> SigPair {
        SigPair {
            pub_key: self.0.clone(),
            signature: self.1.sign(msg)
        }
    }

    #[inline]
    pub fn verify(&self, msg: &[u8], sig: &sign::Signature) -> bool {
        PublicKey::verify(&self.0, msg, sig)
    }

    pub fn gen_keypair() -> KeyPair {
        let mut raw_seed: [u8; sign::SEEDBYTES] = [0; sign::SEEDBYTES];
        randombytes::randombytes_into(&mut raw_seed);
        let seed = sign::Seed::from_slice(&raw_seed).unwrap();
        let (pk, sk) = sign::keypair_from_seed(&seed);
        KeyPair(PublicKey(pk), PrivateKey {
            seed,
            key: sk
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_and_recover_keys() {
        let kp = KeyPair::gen_keypair();

        let pk = &*kp.0.to_wif();
        assert_eq!(&*PublicKey::from_wif(pk).unwrap().to_wif(), pk);

        let sk = &*kp.1.to_wif();
        assert_eq!(&*PrivateKey::from_wif(sk).unwrap().1.to_wif(), sk);
    }

    #[test]
    fn test_import_keys_from_wif() {
        let kp = PrivateKey::from_wif("3GAD3otqozDorfu1iDpMQJ1gzWp8PRFEjVHZivZdedKW3i3KtM").unwrap();
        assert_eq!(&*kp.1.to_wif(), "3GAD3otqozDorfu1iDpMQJ1gzWp8PRFEjVHZivZdedKW3i3KtM");
        assert_eq!(&*kp.0.to_wif(), "GOD52QZDBUStV5CudxvKf6bPsQeN7oeKTkEm2nAU1vAUqNVexGTb8");
    }

    #[test]
    fn test_invalid_prefix() {
        let mut bytes = bs58::decode("3GAD3otqozDorfu1iDpMQJ1gzWp8PRFEjVHZivZdedKW3i3KtM").into_vec().unwrap();
        bytes[0] = 255;
        let wif = bs58::encode(bytes).into_string();
        assert_eq!(PrivateKey::from_wif(&wif).unwrap_err().kind, WifErrorKind::InvalidPrefix);

        let mut bytes = bs58::decode("52QZDBUStV5CudxvKf6bPsQeN7oeKTkEm2nAU1vAUqNVexGTb8").into_vec().unwrap();
        bytes[0] = 255;
        let mut wif = bs58::encode(bytes).into_string();
        wif.insert_str(0, PUB_ADDRESS_PREFIX);
        assert_eq!(PublicKey::from_wif(&wif).unwrap_err().kind, WifErrorKind::InvalidPrefix);
    }

    #[test]
    fn test_invalid_checksum() {
        let mut bytes = bs58::decode("3GAD3otqozDorfu1iDpMQJ1gzWp8PRFEjVHZivZdedKW3i3KtM").into_vec().unwrap();
        let len = bytes.len();
        for i in 1..5 {
            bytes[len - i] = 0;
        }
        let wif = bs58::encode(bytes).into_string();
        assert_eq!(PrivateKey::from_wif(&wif).unwrap_err().kind, WifErrorKind::InvalidChecksum);

        let mut bytes = bs58::decode("52QZDBUStV5CudxvKf6bPsQeN7oeKTkEm2nAU1vAUqNVexGTb8").into_vec().unwrap();
        let len = bytes.len();
        for i in 1..5 {
            bytes[len - i] = 0;
        }
        let mut wif = bs58::encode(bytes).into_string();
        wif.insert_str(0, PUB_ADDRESS_PREFIX);
        assert_eq!(PublicKey::from_wif(&wif).unwrap_err().kind, WifErrorKind::InvalidChecksum);
    }

    #[test]
    fn test_sign_message() {
        let msg = "Hello world!".as_bytes();
        let kp = KeyPair::gen_keypair();

        let sig = &kp.1.sign(msg);
        assert!(kp.0.verify(msg, sig));

        let pair = SigPair {
            pub_key: kp.0,
            signature: *sig
        };
        assert!(pair.verify(msg));

        // Test bad keys
        let kp = KeyPair::gen_keypair();
        assert!(!kp.verify(msg, sig));
    }
}
