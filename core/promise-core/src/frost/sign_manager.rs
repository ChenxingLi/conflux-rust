use std::sync::Arc;

use rand_core::OsRng;

use crate::crypto_types::{
    SignatureShare, SigningCommitments, SigningNonces, SigningShare,
};

use super::{
    FrostEpochState, FrostError, FrostPubKeyContext, NodeID, SignTaskID,
};

/// DANGER: private key.
pub struct SignManager {
    context: Arc<FrostPubKeyContext>,

    current_id: NodeID,
    signing_shares: Vec<SigningShare>,
    signing_nonces: Vec<SigningNonces>,
}

impl SignManager {
    pub fn make_nonce_commitments(
        &mut self, start: usize, length: usize,
    ) -> Vec<SigningCommitments> {
        // The caller guarantee that start + length does not overflow.
        let end = start + length;
        self.generate_nonces(end);

        self.signing_nonces[start..end]
            .iter()
            .map(|x| x.commitments().clone())
            .collect()
    }

    pub fn sign(
        &self, state: &FrostEpochState, sign_task_id: SignTaskID,
    ) -> Result<Option<SignatureShare>, FrostError> {
        assert_eq!(self.context.epoch, state.epoch());

        let sign_task = state.sign_task(sign_task_id)?;
        Ok(sign_task.sign(
            self.current_id,
            &self.signing_shares,
            &self.signing_nonces,
        ))
    }

    fn generate_nonces(&mut self, required_length: usize) {
        if self.signing_nonces.len() >= required_length {
            return;
        }
        let num_generate_nonces = required_length - self.signing_nonces.len();
        let secret = self.signing_shares.first().unwrap();
        for _ in 0..num_generate_nonces {
            let new_nonce = SigningNonces::new(secret, &mut OsRng);
            self.signing_nonces.push(new_nonce);
        }
    }
}
