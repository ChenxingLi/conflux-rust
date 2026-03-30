// Copyright (c) The Diem Core Contributors
// SPDX-License-Identifier: Apache-2.0

// Copyright 2021 Conflux Foundation. All rights reserved.
// Conflux is free software and distributed under GNU General Public License.
// See http://www.gnu.org/licenses/

use crate::config::SecureBackend;
use serde::{Deserialize, Serialize};
use std::{net::SocketAddr, path::PathBuf};

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct ExecutionConfig {
    pub sign_vote_proposal: bool,
    pub service: ExecutionCorrectnessService,
    pub backend: SecureBackend,
    pub network_timeout_ms: u64,
}

impl Default for ExecutionConfig {
    fn default() -> ExecutionConfig {
        ExecutionConfig {
            service: ExecutionCorrectnessService::Thread,
            backend: SecureBackend::InMemoryStorage,
            sign_vote_proposal: true,
            network_timeout_ms: 30_000,
        }
    }
}

impl ExecutionConfig {
    pub fn set_data_dir(&mut self, data_dir: PathBuf) {
        if let SecureBackend::OnDiskStorage(backend) = &mut self.backend {
            backend.set_data_dir(data_dir);
        }
    }
}

/// Defines how execution correctness should be run
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum ExecutionCorrectnessService {
    /// This runs execution correctness in the same thread as event
    /// processor.
    Local,
    /// This is the production, separate service approach
    Process(RemoteExecutionService),
    /// This runs safety rules in the same thread as event processor but
    /// data is passed through the light weight RPC (serializer)
    Serializer,
    /// This creates a separate thread to run execution correctness, it
    /// is similar to a fork / exec style
    Thread,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RemoteExecutionService {
    pub server_address: SocketAddr,
}
