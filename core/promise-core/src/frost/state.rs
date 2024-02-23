use super::{
    node_id, nonce_commitments::EpochNonceCommitments,
    sign_task_manager::SignTaskManager, FrostError, FrostPubKeyContext,
    FrostSignTask, FrostSignerGroup, NodeID, Round, SignTaskID,
};
use crate::crypto_types::{
    Element, NonceCommitment, Signature, SignatureShare,
};
use std::{collections::BTreeMap, sync::Arc};

pub struct FrostEpochState {
    context: Arc<FrostPubKeyContext>,

    signer_group: FrostSignerGroup,
    nonce_commitments: EpochNonceCommitments,

    sign_task_manager: SignTaskManager,

    // None means the current epoch has not start.
    current_round: Option<usize>,
}

pub enum UpdateSignatureShare {
    Pending,
    Retry(SignTaskID),
    Done(Signature),
}

impl FrostEpochState {
    pub fn current_round(&self) -> Result<Round, FrostError> {
        self.current_round.ok_or(FrostError::EpochNotStart)
    }

    pub fn start_round(&mut self, round: Round) {
        if self.current_round.is_none() {
            self.signer_group.update_emulated_verifying_shares();
        }
        self.current_round = Some(round);
    }

    pub fn receive_nonce_commitments(
        &mut self, node_id: NodeID,
        nonce_commitments: Vec<[NonceCommitment; 2]>,
    ) -> Result<(), FrostError> {
        let accept_new_node = self.current_round.is_none();
        self.nonce_commitments.insert_commitments(
            node_id,
            &mut self.signer_group,
            nonce_commitments,
            accept_new_node,
        )?;
        if accept_new_node {
            self.signer_group.insert_node(&node_id);
        }
        Ok(())
    }

    pub fn receive_sign_task(
        &mut self, message: Vec<u8>,
    ) -> Result<SignTaskID, FrostError> {
        let round = self.current_round()?;

        let nonce_commitments = self.nonce_commitments.pull_next_commitments(
            &mut self.signer_group,
            &self.context.identifier_groups,
        )?;

        // Should always success here
        let task = self
            .signer_group
            .make_sign_task(&nonce_commitments, message);

        let id = self.sign_task_manager.insert_sign_task(task, round);

        Ok(id)
    }

    pub fn receive_signature_share(
        &mut self, task_id: SignTaskID, node_id: NodeID, share: SignatureShare,
    ) -> Result<UpdateSignatureShare, FrostError> {
        let sign_task = self.sign_task_manager.get_mut(task_id)?;

        if let Err(e) =
            sign_task.insert_signature_share(&node_id.to_identifier(), share)
        {
            if e == FrostError::InvalidSignatureShare {
                self.signer_group.remove_nodes(&[node_id]);
                let message = sign_task.message().to_vec();
                self.sign_task_manager.remove_failed_sign_task(task_id);

                // Consider retry.
                let retry_id = self.receive_sign_task(message)?;
                return Ok(UpdateSignatureShare::Retry(retry_id));
            }
            return Err(e);
        }

        let maybe_signature = sign_task.try_aggregate_signature_share();
        if let Some(sig) = maybe_signature {
            self.sign_task_manager.complete_sign_task(task_id);
            Ok(UpdateSignatureShare::Done(sig))
        } else {
            Ok(UpdateSignatureShare::Pending)
        }
    }

    pub fn recycle_timeout_sign_tasks(&mut self, latest_round: Round) {
        let removed_sign_tasks = self.sign_task_manager.gc_sign_tasks(latest_round, &mut self.signer_group);
        // TODO: retry timeout sign tasks.
    }
}
