// Copyright 2018 The Exonum Team
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//   http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

use exonum::blockchain::{ExecutionResult, Transaction};
use exonum::crypto::PublicKey;
use exonum::helpers::ValidatorId;
use exonum::messages::Message;
use exonum::storage::Fork;

use btc_transaction_utils::p2wsh::InputSigner;
use btc_transaction_utils::{InputSignature, InputSignatureRef, TxInRef};
use secp256k1::{self, Secp256k1};

use BTC_ANCHORING_SERVICE_ID;
use btc;

use super::BtcAnchoringSchema;
use super::data_layout::TxInputId;

transactions! {
    pub Transactions {
        const SERVICE_ID = BTC_ANCHORING_SERVICE_ID;

        /// Exonum message with the signature for the new anchoring transaction.
        struct Signature {
            /// Public key of validator.
            from: &PublicKey,
            /// Public key index in anchoring public keys list.
            validator: ValidatorId,
            /// Signed transaction.
            tx: btc::Transaction,
            /// Signed input.
            input: u32,
            /// Signature content.
            content: &[u8]
        }
    }
}

// TODO Implement error types.

impl Signature {
    pub fn input_id(&self) -> TxInputId {
        TxInputId {
            txid: self.tx().id(),
            input: self.input(),
        }
    }

    pub fn input_signature(
        &self,
        context: &Secp256k1,
    ) -> Result<InputSignatureRef, secp256k1::Error> {
        InputSignatureRef::from_bytes(context, self.content())
    }
}

impl Transaction for Signature {
    fn verify(&self) -> bool {
        let context = Secp256k1::without_caps();
        self.input_signature(&context).is_ok() && self.verify_signature(self.from())
    }

    fn execute(&self, fork: &mut Fork) -> ExecutionResult {
        let tx = self.tx();
        let mut anchoring_schema = BtcAnchoringSchema::new(fork);
        // We already have enough signatures to spend anchoring transaction.
        if anchoring_schema
            .anchoring_transactions_chain()
            .last()
            .map(|tx| tx.id()) == Some(tx.id())
        {
            return Ok(());
        }

        let anchoring_state = anchoring_schema.actual_state();
        let (expected_transaction, expected_inputs) = anchoring_schema
            .proposed_anchoring_transaction(&anchoring_state)
            .expect(
                "Implement Error code: received signature for the incorrect anchoring transaction",
            )
            .expect("Implement Error code: same as above");
        assert_eq!(
            expected_transaction.id(),
            tx.id(),
            "Implement Error code: expected transaction: {:?}, got: {:?}",
            expected_transaction,
            tx
        );

        let redeem_script = anchoring_state.actual_configuration().redeem_script();
        let redeem_script_content = redeem_script.content();
        let public_key = redeem_script_content
            .public_keys
            .get(self.validator().0 as usize)
            .cloned()
            .expect("Implement Error code: public key of validator is absent");

        let input_signer = InputSigner::new(redeem_script.clone());
        let context = Secp256k1::without_caps();
        // Checks signature content
        let input_signature_ref = self.input_signature(&context).unwrap();
        let input_tx = expected_inputs
            .get(self.input() as usize)
            .expect("Implement Error code: input with the given index doesn't exist");
        input_signer
            .verify_input(
                TxInRef::new(tx.as_ref(), self.input() as usize),
                input_tx.as_ref(),
                &public_key,
                input_signature_ref,
            )
            .expect("Implement Error code: input signature verification failed");

        // Adds signature to schema.
        let input_id = self.input_id();
        let mut input_signatures = anchoring_schema.input_signatures(&input_id, &redeem_script);
        if input_signatures.len() != redeem_script_content.quorum {
            input_signatures.insert(self.validator(), self.content().to_vec());
            anchoring_schema
                .transaction_signatures_mut()
                .put(&input_id, input_signatures);
        }
        // Tries to finalize transaction.
        let mut tx: btc::Transaction = tx;
        for index in 0..expected_inputs.len() {
            let input_id = TxInputId::new(self.tx().id(), index as u32);
            let input_signatures = anchoring_schema.input_signatures(&input_id, &redeem_script);

            if input_signatures.len() != redeem_script_content.quorum {
                return Ok(());
            }

            input_signer.spend_input(
                &mut tx.0.input[index],
                input_signatures
                    .into_iter()
                    .map(|bytes| InputSignature::from_bytes(&context, bytes).unwrap()),
            );
        }

        let payload = tx.anchoring_metadata().unwrap().1;
        
        info!("====== ANCHORING ======");
        info!("txid: {}", tx.id().to_string(),);
        info!("height: {}", payload.block_height,);
        info!("hash: {}", payload.block_hash,);
        info!("balance: {}", tx.0.output[0].value,);
        trace!("Anchoring txhex: {}", tx.to_string());

        // Adds finalized transaction to the tail of anchoring transactions.
        anchoring_schema.anchoring_transactions_chain_mut().push(tx);
        if let Some(unspent_funding_tx) = anchoring_schema.unspent_funding_transaction() {
            anchoring_schema
                .spent_funding_transactions_mut()
                .put(&unspent_funding_tx.id(), unspent_funding_tx);
        }

        Ok(())
    }
}
