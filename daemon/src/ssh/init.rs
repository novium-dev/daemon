/*
 * Copyright (C) 2025 Markus Probst
 * This program is free software: you can redistribute it and/or modify
 * it under the terms of the GNU Affero General Public License as published by
 * the Free Software Foundation, version 3.
 *
 * This program is distributed in the hope that it will be useful,
 * but WITHOUT ANY WARRANTY; without even the implied warranty of
 * MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
 * GNU Affero General Public License for more details.
 *
 * You should have received a copy of the GNU Affero General Public License
 * along with this program. If not, see <https://www.gnu.org/licenses/>.
 */
use crate::{Daemon, DaemonExtra, GIT_HASH, common};
use core::time::Duration;
use russh::server::*;
use russh::*;
use std::sync::Arc;
use tracing::error;

impl<T: DaemonExtra> Daemon<T> {
    pub async fn serve_ssh(&self) -> Result<(), anyhow::Error> {
        if let Some(ssh) = self.config.ssh.clone() {
            let mut keys = Vec::with_capacity(ssh.host_key.len());
            for key in &ssh.host_key {
                keys.push(
                    key.as_russh_key()
                        .await
                        .inspect_err(|err| error!("Failed to start ssh server (key): {:?}", err))?,
                );
                tracing::debug!("Loaded ssh key: {:?}", key);
            }
            let mut methods = MethodSet::empty();
            methods.push(MethodKind::Password);
            let ssh_config = Arc::new(Config {
                keys,
                server_id: SshId::Standard(format!("{}{}", common::SSH_BANNER_PREFIX, GIT_HASH)),
                methods,
                ..Config::default()
            });
            let mut server_task = self.server_task_cancellable.lock().await;
            for listener in ssh.listeners {
                let daemon = self.clone();
                let ssh_config = Arc::clone(&ssh_config);
                server_task.spawn(async move {
                    if let Err(err) = (crate::ssh::server::DaemonSSHServer {
                        cache_invalidation: Duration::from_millis(ssh.auth_cache_duration),
                        daemon: daemon.clone(),
                    })
                    .run_on_address(ssh_config, &listener)
                    .await
                    {
                        error!("Failed to start ssh server (bind): {:?}", err);
                    }
                });
            }
        }
        Ok(())
    }
}
