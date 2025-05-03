use crate::{Daemon, GIT_HASH, common};
use russh::server::*;
use russh::*;
use std::sync::Arc;
use std::time::Duration;
use tracing::error;

impl Daemon {
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
            for listener in ssh.listeners {
                let daemon = self.clone();
                let ssh_config = ssh_config.clone();
                tokio::task::spawn(async move {
                    if let Err(err) = (crate::ssh::server::DaemonSSHServer {
                        cache_invalidation: Duration::from_millis(ssh.auth_cache_duration),
                        state: daemon.clone(),
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
