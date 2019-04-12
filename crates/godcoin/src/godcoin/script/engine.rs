use sodiumoxide::crypto::sign;
use std::borrow::Cow;

use super::stack::*;
use super::*;
use crate::crypto::PublicKey;
use crate::tx::TxVariant;

macro_rules! map_err_type {
    ($self:expr, $var:expr) => {
        $var.map_err(|e| $self.new_err(e))
    };
}

pub struct ScriptEngine<'a> {
    script: Cow<'a, Script>,
    tx: Cow<'a, TxVariant>,
    pos: usize,
    stack: Stack,
}

impl<'a> ScriptEngine<'a> {
    pub fn checked_new<T, S>(tx: T, script: S) -> Option<Self>
    where
        T: Into<Cow<'a, TxVariant>>,
        S: Into<Cow<'a, Script>>,
    {
        let script = script.into();
        let tx = tx.into();
        if script.len() > MAX_BYTE_SIZE {
            return None;
        }
        Some(Self {
            script,
            tx,
            pos: 0,
            stack: Stack::new(),
        })
    }

    pub fn eval(&mut self) -> Result<bool, EvalErr> {
        self.pos = 0;
        let mut if_marker = 0;
        let mut ignore_else = false;
        while let Some(op) = self.consume_op()? {
            match op {
                // Stack manipulation
                OpFrame::OpNot => {
                    let b = map_err_type!(self, self.stack.pop_bool())?;
                    map_err_type!(self, self.stack.push(!b))?;
                }
                // Control
                OpFrame::OpIf => {
                    if_marker += 1;
                    ignore_else = map_err_type!(self, self.stack.pop_bool())?;
                    if ignore_else {
                        continue;
                    }
                    let req_if_marker = if_marker;
                    self.consume_op_until(|op| {
                        if op == OpFrame::OpIf {
                            if_marker += 1;
                            false
                        } else if op == OpFrame::OpElse {
                            if_marker == req_if_marker
                        } else if op == OpFrame::OpEndIf {
                            let do_break = if_marker == req_if_marker;
                            if_marker -= 1;
                            do_break
                        } else {
                            false
                        }
                    })?;
                }
                OpFrame::OpElse => {
                    if !ignore_else {
                        continue;
                    }
                    let req_if_marker = if_marker;
                    self.consume_op_until(|op| {
                        if op == OpFrame::OpIf {
                            if_marker += 1;
                            false
                        } else if op == OpFrame::OpElse {
                            if_marker == req_if_marker
                        } else if op == OpFrame::OpEndIf {
                            let do_break = if_marker == req_if_marker;
                            if_marker -= 1;
                            do_break
                        } else {
                            false
                        }
                    })?;
                }
                OpFrame::OpEndIf => {
                    if_marker -= 1;
                }
                OpFrame::OpReturn => {
                    if_marker = 0;
                    break;
                }
                // Crypto
                OpFrame::OpCheckSig => {
                    let key = map_err_type!(self, self.stack.pop_pubkey())?;
                    let mut buf = Vec::with_capacity(4096);
                    self.tx.encode(&mut buf);
                    let mut success = false;
                    for pair in &self.tx.signature_pairs {
                        if key == pair.pub_key {
                            if key.verify(&buf, &pair.signature) {
                                success = true;
                            }
                            break;
                        }
                    }
                    map_err_type!(self, self.stack.push(success))?;
                }
                OpFrame::OpCheckMultiSig(threshold, key_count) => {
                    // Keys need to be popped off the stack first before doing anything else
                    // to ensure the stack is in a valid state if anything fails.
                    let keys = {
                        let mut vec = Vec::with_capacity(usize::from(key_count));
                        for _ in 0..key_count {
                            vec.push(map_err_type!(self, self.stack.pop_pubkey())?);
                        }
                        vec
                    };

                    if threshold == 0 {
                        map_err_type!(self, self.stack.push(true))?;
                        continue;
                    } else if usize::from(threshold) > keys.len() {
                        map_err_type!(self, self.stack.push(false))?;
                        continue;
                    }

                    let mut buf = Vec::with_capacity(4096);
                    self.tx.encode(&mut buf);

                    let mut valid_threshold = 0;
                    let mut success = true;
                    'key_loop: for key in &keys {
                        for pair in &self.tx.signature_pairs {
                            if key == &pair.pub_key {
                                let sig_verified = key.verify(&buf, &pair.signature);
                                if sig_verified {
                                    valid_threshold += 1;
                                    continue 'key_loop;
                                } else {
                                    success = false;
                                    break 'key_loop;
                                }
                            }
                        }
                    }
                    if success {
                        map_err_type!(self, self.stack.push(valid_threshold >= threshold))?;
                    } else {
                        map_err_type!(self, self.stack.push(false))?;
                    }
                }
                // Handle push ops
                _ => {
                    map_err_type!(self, self.stack.push(op))?;
                }
            }
        }

        if if_marker > 0 {
            return Err(self.new_err(EvalErrType::UnexpectedEOF));
        }

        // Scripts must return true or false
        map_err_type!(self, self.stack.pop_bool())
    }

    fn consume_op_until<F>(&mut self, mut matcher: F) -> Result<(), EvalErr>
    where
        F: FnMut(OpFrame) -> bool,
    {
        loop {
            match self.consume_op()? {
                Some(op) => {
                    if matcher(op) {
                        break;
                    }
                }
                None => return Err(self.new_err(EvalErrType::UnexpectedEOF)),
            }
        }

        Ok(())
    }

    fn consume_op(&mut self) -> Result<Option<OpFrame>, EvalErr> {
        if self.pos == self.script.len() {
            return Ok(None);
        }
        let byte = self.script[self.pos];
        self.pos += 1;

        match byte {
            // Push value
            o if o == Operand::PushFalse as u8 => Ok(Some(OpFrame::False)),
            o if o == Operand::PushTrue as u8 => Ok(Some(OpFrame::True)),
            o if o == Operand::PushPubKey as u8 => {
                let slice = self.pos..self.pos + sign::PUBLICKEYBYTES;
                let key = match self.script.get(slice) {
                    Some(slice) => {
                        self.pos += sign::PUBLICKEYBYTES;
                        PublicKey::from_slice(slice).unwrap()
                    }
                    None => {
                        return Err(self.new_err(EvalErrType::UnexpectedEOF));
                    }
                };
                Ok(Some(OpFrame::PubKey(key)))
            }
            // Stack manipulation
            o if o == Operand::OpNot as u8 => Ok(Some(OpFrame::OpNot)),
            // Control
            o if o == Operand::OpIf as u8 => Ok(Some(OpFrame::OpIf)),
            o if o == Operand::OpElse as u8 => Ok(Some(OpFrame::OpElse)),
            o if o == Operand::OpEndIf as u8 => Ok(Some(OpFrame::OpEndIf)),
            o if o == Operand::OpReturn as u8 => Ok(Some(OpFrame::OpReturn)),
            // Crypto
            o if o == Operand::OpCheckSig as u8 => Ok(Some(OpFrame::OpCheckSig)),
            o if o == Operand::OpCheckMultiSig as u8 => {
                let threshold = match self.script.get(self.pos) {
                    Some(threshold) => {
                        self.pos += 1;
                        *threshold
                    }
                    None => {
                        return Err(self.new_err(EvalErrType::UnexpectedEOF));
                    }
                };

                let key_count = match self.script.get(self.pos) {
                    Some(key_count) => {
                        self.pos += 1;
                        *key_count
                    }
                    None => {
                        return Err(self.new_err(EvalErrType::UnexpectedEOF));
                    }
                };

                Ok(Some(OpFrame::OpCheckMultiSig(threshold, key_count)))
            }
            _ => Err(self.new_err(EvalErrType::UnknownOp)),
        }
    }

    fn new_err(&self, err: EvalErrType) -> EvalErr {
        EvalErr::new(self.pos, err)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::{KeyPair, SigPair};
    use crate::tx::{SignTx, TransferTx, Tx, TxType};

    #[test]
    fn true_only_script() {
        let mut engine = new_engine(Builder::new().push(OpFrame::True));
        assert!(engine.eval().unwrap());
        assert!(engine.stack.is_empty());
    }

    #[test]
    fn false_only_script() {
        let mut engine = new_engine(Builder::new().push(OpFrame::False));
        assert!(!engine.eval().unwrap());
        assert!(engine.stack.is_empty());
    }

    #[test]
    fn if_script() {
        #[rustfmt::skip]
        let mut engine = new_engine(
            Builder::new()
                .push(OpFrame::True)
                .push(OpFrame::OpIf)
                    .push(OpFrame::False)
                .push(OpFrame::OpEndIf),
        );
        assert!(!engine.eval().unwrap());
        assert!(engine.stack.is_empty());

        #[rustfmt::skip]
        let mut engine = new_engine(
            Builder::new()
                .push(OpFrame::True)
                .push(OpFrame::OpIf)
                    .push(OpFrame::True)
                .push(OpFrame::OpEndIf),
        );
        assert!(engine.eval().unwrap());
        assert!(engine.stack.is_empty());
    }

    #[test]
    fn if_script_with_ret() {
        #[rustfmt::skip]
        let mut engine = new_engine(
            Builder::new()
                .push(OpFrame::True)
                .push(OpFrame::OpIf)
                    .push(OpFrame::False)
                    .push(OpFrame::OpReturn)
                .push(OpFrame::OpEndIf)
                .push(OpFrame::True),
        );
        assert!(!engine.eval().unwrap());
        assert!(engine.stack.is_empty());

        #[rustfmt::skip]
        let mut engine = new_engine(
            Builder::new()
                .push(OpFrame::False)
                .push(OpFrame::OpIf)
                    .push(OpFrame::False)
                .push(OpFrame::OpElse)
                    .push(OpFrame::True)
                    .push(OpFrame::OpReturn)
                .push(OpFrame::OpEndIf)
                .push(OpFrame::False),
        );
        assert!(engine.eval().unwrap());
        assert!(engine.stack.is_empty());
    }

    #[test]
    fn branch_if() {
        #[rustfmt::skip]
        let mut engine = new_engine(
            Builder::new()
                .push(OpFrame::True)
                .push(OpFrame::OpIf)
                    .push(OpFrame::True)
                .push(OpFrame::OpElse)
                    .push(OpFrame::False)
                .push(OpFrame::OpEndIf),
        );
        assert!(engine.eval().unwrap());
        assert!(engine.stack.is_empty());

        #[rustfmt::skip]
        let mut engine = new_engine(
            Builder::new()
                .push(OpFrame::False)
                .push(OpFrame::OpIf)
                    .push(OpFrame::False)
                .push(OpFrame::OpElse)
                    .push(OpFrame::True)
                .push(OpFrame::OpEndIf),
        );
        assert!(engine.eval().unwrap());
        assert!(engine.stack.is_empty());
    }

    #[test]
    fn nested_branch_if() {
        #[rustfmt::skip]
        let mut engine = new_engine(
            Builder::new()
                .push(OpFrame::True)
                .push(OpFrame::OpIf)
                    .push(OpFrame::True)
                    .push(OpFrame::OpIf)
                        .push(OpFrame::True)
                    .push(OpFrame::OpEndIf)
                .push(OpFrame::OpElse)
                    .push(OpFrame::False)
                    .push(OpFrame::OpIf)
                        .push(OpFrame::False)
                    .push(OpFrame::OpEndIf)
                .push(OpFrame::OpEndIf),
        );
        assert!(engine.eval().unwrap());
        assert!(engine.stack.is_empty());

        #[rustfmt::skip]
        let mut engine = new_engine(
            Builder::new()
                .push(OpFrame::False)
                .push(OpFrame::OpIf)
                    .push(OpFrame::True)
                    .push(OpFrame::OpIf)
                        .push(OpFrame::False)
                    .push(OpFrame::OpEndIf)
                .push(OpFrame::OpElse)
                    .push(OpFrame::True)
                    .push(OpFrame::OpIf)
                        .push(OpFrame::True)
                    .push(OpFrame::OpEndIf)
                .push(OpFrame::OpEndIf),
        );
        assert!(engine.eval().unwrap());
        assert!(engine.stack.is_empty());
    }

    #[test]
    fn fail_invalid_stack_on_return() {
        let key = KeyPair::gen_keypair().0;
        let mut engine = new_engine(Builder::new().push(OpFrame::PubKey(key)));
        assert_eq!(
            engine.eval().unwrap_err().err,
            EvalErrType::InvalidItemOnStack
        );
    }

    #[test]
    fn fail_invalid_if_cmp() {
        let key = KeyPair::gen_keypair().0;
        let mut engine = new_engine(
            Builder::new()
                .push(OpFrame::PubKey(key))
                .push(OpFrame::OpIf),
        );
        assert_eq!(
            engine.eval().unwrap_err().err,
            EvalErrType::InvalidItemOnStack
        );
    }

    #[test]
    fn fail_unended_if() {
        let mut engine = new_engine(Builder::new().push(OpFrame::True).push(OpFrame::OpIf));
        assert_eq!(engine.eval().unwrap_err().err, EvalErrType::UnexpectedEOF);

        let mut engine = new_engine(Builder::new().push(OpFrame::False).push(OpFrame::OpIf));
        assert_eq!(engine.eval().unwrap_err().err, EvalErrType::UnexpectedEOF);
    }

    #[test]
    fn checksig() {
        let key = KeyPair::gen_keypair();
        let mut engine = new_engine_with_signers(
            &[key.clone()],
            Builder::new()
                .push(OpFrame::PubKey(key.0.clone()))
                .push(OpFrame::OpCheckSig),
        );
        assert!(engine.eval().unwrap());

        let other = KeyPair::gen_keypair();
        let mut engine = new_engine_with_signers(
            &[key.clone()],
            Builder::new()
                .push(OpFrame::PubKey(other.0.clone()))
                .push(OpFrame::OpCheckSig),
        );
        assert!(!engine.eval().unwrap());

        let mut engine = new_engine_with_signers(
            &[other],
            Builder::new()
                .push(OpFrame::PubKey(key.0))
                .push(OpFrame::OpCheckSig),
        );
        assert!(!engine.eval().unwrap());
    }

    #[test]
    fn checkmultisig_equal_threshold() {
        let key_1 = KeyPair::gen_keypair();
        let key_2 = KeyPair::gen_keypair();
        let key_3 = KeyPair::gen_keypair();

        let mut engine = new_engine_with_signers(
            &[key_3.clone(), key_1.clone()],
            Builder::new()
                .push(OpFrame::PubKey(key_1.0.clone()))
                .push(OpFrame::PubKey(key_2.0.clone()))
                .push(OpFrame::PubKey(key_3.0.clone()))
                .push(OpFrame::OpCheckMultiSig(2, 3)),
        );
        assert!(engine.eval().unwrap());
    }

    #[test]
    fn checkmultisig_threshold_unmet() {
        let key_1 = KeyPair::gen_keypair();
        let key_2 = KeyPair::gen_keypair();
        let key_3 = KeyPair::gen_keypair();

        let mut engine = new_engine_with_signers(
            &[key_3.clone(), key_1.clone()],
            Builder::new()
                .push(OpFrame::PubKey(key_1.0.clone()))
                .push(OpFrame::PubKey(key_2.0.clone()))
                .push(OpFrame::PubKey(key_3.0.clone()))
                .push(OpFrame::OpCheckMultiSig(3, 3)),
        );
        assert!(!engine.eval().unwrap());
    }

    #[test]
    fn checkmultisig_invalid_sig() {
        let key_1 = KeyPair::gen_keypair();
        let key_2 = KeyPair::gen_keypair();
        let key_3 = KeyPair::gen_keypair();

        let mut engine = new_engine_with_signers(
            &[key_1.clone(), key_2.clone(), KeyPair::gen_keypair()],
            Builder::new()
                .push(OpFrame::PubKey(key_1.0.clone()))
                .push(OpFrame::PubKey(key_2.0.clone()))
                .push(OpFrame::PubKey(key_3.0.clone()))
                .push(OpFrame::OpCheckMultiSig(2, 3)),
        );
        // This should evaluate to true as the threshold is met, any other invalid signatures are
        // no longer relevant. There is no incentive to inject fake signatures unless the
        // broadcaster wants to pay more in fees.
        assert!(engine.eval().unwrap());

        let mut engine = {
            let to = KeyPair::gen_keypair();
            let script = Builder::new()
                .push(OpFrame::PubKey(key_1.0.clone()))
                .push(OpFrame::PubKey(key_2.0.clone()))
                .push(OpFrame::PubKey(key_3.0.clone()))
                .push(OpFrame::OpCheckMultiSig(2, 3))
                .build();

            let mut tx = TransferTx {
                base: Tx {
                    tx_type: TxType::TRANSFER,
                    timestamp: 1500000000,
                    fee: "1 GOLD".parse().unwrap(),
                    signature_pairs: vec![SigPair {
                        // Test valid key with invalid signature
                        pub_key: key_2.0.clone(),
                        signature: sign::Signature([0; sign::SIGNATUREBYTES]),
                    }],
                },
                from: key_1.clone().0.into(),
                to: to.clone().0.into(),
                amount: "10 GOLD".parse().unwrap(),
                script: script.clone(),
                memo: vec![],
            };
            tx.append_sign(&key_1);

            ScriptEngine::checked_new(TxVariant::TransferTx(tx), script).unwrap()
        };
        assert!(!engine.eval().unwrap());
    }

    #[test]
    fn checksig_and_checkmultisig_with_if() {
        let key_0 = KeyPair::gen_keypair();
        let key_1 = KeyPair::gen_keypair();
        let key_2 = KeyPair::gen_keypair();
        let key_3 = KeyPair::gen_keypair();

        // Test threshold is met and tx is signed with key_0
        #[rustfmt::skip]
        let mut engine = new_engine_with_signers(
            &[key_0.clone(), key_1.clone(), key_2.clone()],
            Builder::new()
                .push(OpFrame::PubKey(key_0.0.clone()))
                .push(OpFrame::OpCheckSig)
                .push(OpFrame::OpIf)
                    .push(OpFrame::PubKey(key_1.0.clone()))
                    .push(OpFrame::PubKey(key_2.0.clone()))
                    .push(OpFrame::PubKey(key_3.0.clone()))
                    .push(OpFrame::OpCheckMultiSig(2, 3))
                    .push(OpFrame::OpReturn)
                .push(OpFrame::OpEndIf)
                .push(OpFrame::False),
        );
        assert!(engine.eval().unwrap());

        // Test tx must be signed with key_0 but threshold is met
        #[rustfmt::skip]
        let mut engine = new_engine_with_signers(
            &[key_1.clone(), key_2.clone()],
            Builder::new()
                .push(OpFrame::PubKey(key_0.0.clone()))
                .push(OpFrame::OpCheckSig)
                .push(OpFrame::OpIf)
                    .push(OpFrame::PubKey(key_1.0.clone()))
                    .push(OpFrame::PubKey(key_2.0.clone()))
                    .push(OpFrame::PubKey(key_3.0.clone()))
                    .push(OpFrame::OpCheckMultiSig(2, 3))
                    .push(OpFrame::OpReturn)
                .push(OpFrame::OpEndIf)
                .push(OpFrame::False),
        );
        assert!(!engine.eval().unwrap());

        // Test multisig threshold not met
        #[rustfmt::skip]
        let mut engine = new_engine_with_signers(
            &[key_0.clone(), key_1.clone()],
            Builder::new()
                .push(OpFrame::PubKey(key_0.0.clone()))
                .push(OpFrame::OpCheckSig)
                .push(OpFrame::OpIf)
                    .push(OpFrame::PubKey(key_1.0.clone()))
                    .push(OpFrame::PubKey(key_2.0.clone()))
                    .push(OpFrame::PubKey(key_3.0.clone()))
                    .push(OpFrame::OpCheckMultiSig(2, 3))
                    .push(OpFrame::OpReturn)
                .push(OpFrame::OpEndIf)
                .push(OpFrame::False),
        );
        assert!(!engine.eval().unwrap());
    }

    #[test]
    fn checksig_and_checkmultisig_with_if_not() {
        let key_0 = KeyPair::gen_keypair();
        let key_1 = KeyPair::gen_keypair();
        let key_2 = KeyPair::gen_keypair();
        let key_3 = KeyPair::gen_keypair();

        // Test threshold is met and tx is signed with key_0
        #[rustfmt::skip]
        let mut engine = new_engine_with_signers(
            &[key_0.clone(), key_1.clone(), key_2.clone()],
            Builder::new()
                .push(OpFrame::PubKey(key_0.0.clone()))
                .push(OpFrame::OpCheckSig)
                .push(OpFrame::OpNot)
                .push(OpFrame::OpIf)
                    .push(OpFrame::False)
                    .push(OpFrame::OpReturn)
                .push(OpFrame::OpEndIf)
                .push(OpFrame::PubKey(key_1.0.clone()))
                .push(OpFrame::PubKey(key_2.0.clone()))
                .push(OpFrame::PubKey(key_3.0.clone()))
                .push(OpFrame::OpCheckMultiSig(2, 3))
        );
        assert!(engine.eval().unwrap());

        // Test tx must be signed with key_0 but threshold is met
        #[rustfmt::skip]
        let mut engine = new_engine_with_signers(
            &[key_1.clone(), key_2.clone()],
            Builder::new()
                .push(OpFrame::PubKey(key_0.0.clone()))
                .push(OpFrame::OpCheckSig)
                .push(OpFrame::OpNot)
                .push(OpFrame::OpIf)
                    .push(OpFrame::False)
                    .push(OpFrame::OpReturn)
                .push(OpFrame::OpEndIf)
                .push(OpFrame::PubKey(key_1.0.clone()))
                .push(OpFrame::PubKey(key_2.0.clone()))
                .push(OpFrame::PubKey(key_3.0.clone()))
                .push(OpFrame::OpCheckMultiSig(2, 3))
        );
        assert!(!engine.eval().unwrap());

        // Test multisig threshold not met
        #[rustfmt::skip]
        let mut engine = new_engine_with_signers(
            &[key_0.clone(), key_1.clone()],
            Builder::new()
                .push(OpFrame::PubKey(key_0.0.clone()))
                .push(OpFrame::OpCheckSig)
                .push(OpFrame::OpNot)
                .push(OpFrame::OpIf)
                    .push(OpFrame::False)
                    .push(OpFrame::OpReturn)
                .push(OpFrame::OpEndIf)
                .push(OpFrame::PubKey(key_1.0.clone()))
                .push(OpFrame::PubKey(key_2.0.clone()))
                .push(OpFrame::PubKey(key_3.0.clone()))
                .push(OpFrame::OpCheckMultiSig(2, 3))
        );
        assert!(!engine.eval().unwrap());
    }

    fn new_engine<'a>(builder: Builder) -> ScriptEngine<'a> {
        let from = KeyPair::gen_keypair();
        new_engine_with_signers(&[from], builder)
    }

    fn new_engine_with_signers<'a>(keys: &[KeyPair], b: Builder) -> ScriptEngine<'a> {
        let to = KeyPair::gen_keypair();
        let script = b.build();

        let mut tx = TransferTx {
            base: Tx {
                tx_type: TxType::TRANSFER,
                timestamp: 1500000000,
                fee: "1 GOLD".parse().unwrap(),
                signature_pairs: vec![],
            },
            from: keys[0].clone().0.into(),
            to: to.clone().0.into(),
            amount: "10 GOLD".parse().unwrap(),
            script: script.clone(),
            memo: vec![],
        };
        for key in keys {
            tx.append_sign(&key);
        }

        ScriptEngine::checked_new(TxVariant::TransferTx(tx), script).unwrap()
    }
}
