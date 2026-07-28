// SPDX-License-Identifier: AGPL-3.0-only
#![expect(clippy::option_if_let_else, reason = "caused by macro")]

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct TaskDescriptor<Task: Send + Sync + 'static> {
    pub id: u64,
    pub time: i64,
    pub write: bool,
    #[serde(flatten)]
    pub data: Task,
}
