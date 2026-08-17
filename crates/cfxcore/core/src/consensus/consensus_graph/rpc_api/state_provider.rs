use crate::errors::{invalid_params_check, Result as CoreResult};

use cfx_statedb::StateDb;
use cfx_storage::{state::StorageView, OpenOptions};
use cfx_types::{Space, H256};

use primitives::EpochNumber;

use super::super::ConsensusGraph;

impl ConsensusGraph {
    /// The pivot hash of the epoch a state query names, for a caller which
    /// opens the state itself. The epoch number is validated, turned into a
    /// height and then into the hash, and the state at that height is checked
    /// against the availability boundary, which is the whole of what consensus
    /// knows about the question. Answering with the hash instead of with a
    /// state object keeps the engine out of consensus for the queries which
    /// need a concrete engine, `cfx_getStorageRoot` among them.
    pub fn get_state_epoch_hash_by_epoch_number(
        &self, epoch_number: EpochNumber, rpc_param_name: &str,
    ) -> CoreResult<H256> {
        invalid_params_check(
            rpc_param_name,
            self.validate_stated_epoch(&epoch_number),
        )?;
        let height = invalid_params_check(
            rpc_param_name,
            self.get_height_from_epoch_number(epoch_number),
        )?;
        let hash =
            self.inner.read().get_pivot_hash_from_epoch_number(height)?;
        // Keep the lock until the answer is out, otherwise the state may
        // expire between the check and the caller's open.
        let state_availability_boundary =
            self.data_man.state_availability_boundary.read();
        if !state_availability_boundary.check_availability(height, &hash) {
            debug!(
                "State for epoch (number={:?} hash={:?}) does not exist: out-of-bound {:?}",
                height, hash, state_availability_boundary
            );
            bail!(format!(
                "State for epoch (number={:?} hash={:?}) does not exist: out-of-bound {:?}",
                height, hash, state_availability_boundary
            ));
        }
        Ok(hash)
    }

    pub fn get_eth_state_db_by_epoch_number(
        &self, epoch_number: EpochNumber, rpc_param_name: &str,
    ) -> CoreResult<StateDb> {
        self.get_state_db_by_epoch_number_with_space(
            epoch_number,
            rpc_param_name,
            Some(Space::Ethereum),
        )
    }

    pub fn get_state_db_by_epoch_number(
        &self, epoch_number: EpochNumber, rpc_param_name: &str,
    ) -> CoreResult<StateDb> {
        self.get_state_db_by_epoch_number_with_space(
            epoch_number,
            rpc_param_name,
            None,
        )
    }

    fn get_state_db_by_epoch_number_with_space(
        &self, epoch_number: EpochNumber, rpc_param_name: &str,
        space: Option<Space>,
    ) -> CoreResult<StateDb> {
        invalid_params_check(
            rpc_param_name,
            self.validate_stated_epoch(&epoch_number),
        )?;
        let height = invalid_params_check(
            rpc_param_name,
            self.get_height_from_epoch_number(epoch_number),
        )?;
        let hash =
            self.inner.read().get_pivot_hash_from_epoch_number(height)?;
        Ok(StateDb::new_readonly(
            self.get_state_by_height_and_hash(height, &hash, space)?,
        ))
    }

    fn get_state_by_height_and_hash(
        &self, height: u64, hash: &H256, space: Option<Space>,
    ) -> CoreResult<Box<dyn StorageView>> {
        // Keep the lock until we get the desired State, otherwise the State may
        // expire.
        let state_availability_boundary =
            self.data_man.state_availability_boundary.read();
        if !state_availability_boundary
            .check_read_availability(height, &hash, space)
        {
            debug!(
                "State for epoch (number={:?} hash={:?}) does not exist: out-of-bound {:?}",
                height, hash, state_availability_boundary
            );
            bail!(format!(
                "State for epoch (number={:?} hash={:?}) does not exist: out-of-bound {:?}",
                height, hash, state_availability_boundary
            ));
        }
        let maybe_state = self
            .data_man
            .storage_manager
            .open_state(
                &hash,
                OpenOptions::read_only()
                    .with_try_open(true)
                    .with_space(space),
            )
            .map_err(|e| format!("Error to get state, err={:?}", e))?;

        let state = match maybe_state {
            Some(state) => state,
            None => {
                bail!(format!(
                    "State for epoch (number={:?} hash={:?}) does not exist",
                    height, hash
                ));
            }
        };

        Ok(state)
    }
}
