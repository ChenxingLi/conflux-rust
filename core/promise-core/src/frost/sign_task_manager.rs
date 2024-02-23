use std::{
    collections::{btree_map::Entry, BTreeMap, BTreeSet, VecDeque},
    process::id,
};

use serde::{Deserialize, Serialize};

use super::{
    error::FrostError, FrostSignTask, FrostSignerGroup, NodeID, Round,
};

#[derive(
    Clone,
    Copy,
    Debug,
    Serialize,
    Deserialize,
    Hash,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
)]
pub struct SignTaskID(usize);

pub struct SignTaskManager {
    next_sign_task_id: SignTaskID,
    open_sign_tasks: BTreeMap<SignTaskID, FrostSignTask>,
    timeout_info: BTreeMap<Round, Vec<SignTaskID>>,
}

impl SignTaskManager {
    pub fn complete_sign_task(&mut self, id: SignTaskID) {
        self.open_sign_tasks.remove(&id);
    }

    // Maybe we have a different way to deal with complete and fail later.
    pub fn remove_failed_sign_task(&mut self, id: SignTaskID) {
        self.open_sign_tasks.remove(&id);
    }

    pub fn insert_sign_task(
        &mut self, task: FrostSignTask, timeout_round: Round,
    ) -> SignTaskID {
        let id = self.next_sign_task_id;
        self.next_sign_task_id.0 += 1;

        self.open_sign_tasks.insert(id, task);

        match self.timeout_info.entry(timeout_round) {
            Entry::Vacant(e) => {
                e.insert(vec![id]);
            }
            Entry::Occupied(e) => {
                e.into_mut().push(id);
            }
        }
        id
    }

    pub fn get_mut(
        &mut self, id: SignTaskID,
    ) -> Result<&mut FrostSignTask, FrostError> {
        self.open_sign_tasks
            .get_mut(&id)
            .ok_or(FrostError::UnknownSignTask)
    }

    pub fn gc_sign_tasks(
        &mut self, round: Round, signer_group: &mut FrostSignerGroup,
    ) -> Vec<(SignTaskID, FrostSignTask)> {
        let mut removed = self.timeout_info.split_off(&(round + 1));
        std::mem::swap(&mut removed, &mut self.timeout_info);

        let timeout_tasks: Vec<SignTaskID> = removed
            .into_iter()
            .map(|(_, ids)| ids.into_iter())
            .flatten()
            .filter(|id| self.open_sign_tasks.contains_key(&id))
            .collect();
        let mut unsigned_identifiers = BTreeSet::new();
        let mut recycled_tasks = vec![];

        for task_id in timeout_tasks.iter() {
            let sign_task = self.open_sign_tasks.remove(task_id).unwrap();
            for identifier in sign_task.unsigned_nodes() {
                unsigned_identifiers.insert(identifier);
            }
            recycled_tasks.push((*task_id, sign_task));
        }

        let timeout_nodes: Vec<NodeID> = signer_group
            .valid_nodes()
            .filter(|x| !unsigned_identifiers.contains(&x.to_identifier()))
            .collect();
        signer_group.remove_nodes(&timeout_nodes);

        recycled_tasks
    }
}
