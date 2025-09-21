// SPDX-License-Identifier: AGPL-3.0-only
use bollard::Docker;
use bollard::models::{Ipam, IpamConfig, NetworkCreateRequest, NetworkDisconnectRequest};
use bollard::query_parameters::InspectNetworkOptions;
use daemon_common::service::error_docker_failure::FromDockerFailure;
use sqlx::{Error, Executor, MySql, Transaction, query, query_as};
use tracing::error;

#[derive(Debug, Clone, Eq, PartialEq, sqlx::FromRow)]
pub struct Network {
    pub id: u32,
    pub ipv4: u16,
    pub ipv6: u16,
}

impl Network {
    pub async fn create<E: From<sqlx::Error> + FromDockerFailure>(
        docker: &Docker,
        tx: &mut Transaction<'_, MySql>,
        ipv4: u16,
        ipv6: u16,
    ) -> Result<Self, E> {
        let id = query!(
            "INSERT INTO `networks` (`ipv4`, `ipv6`) VALUES (?, ?)",
            ipv4,
            ipv6
        )
        .execute(&mut **tx)
        .await
        .inspect_err(|err| {
            if let Error::Database(db) = &err
                && (db.is_unique_violation() || db.is_check_violation())
            {
                return;
            }
            error!("Failed to insert network: {err:?}");
        })?
        .last_insert_id() as u32;

        let ipv4_split = [(ipv4 >> 8u16) as u8, ipv4 as u8];

        docker
            .create_network(NetworkCreateRequest {
                name: format!("{}{}", crate::common::NETWORK_PREFIX, id),
                enable_ipv6: Some(true),
                internal: Some(true),
                ipam: Some(Ipam {
                    config: Some(vec![
                        IpamConfig {
                            subnet: Some(format!(
                                "{}.{}.{}.{}/26",
                                crate::common::IPV4_PREFIX[0],
                                crate::common::IPV4_PREFIX[1],
                                ipv4_split[0],
                                ipv4_split[1]
                            )),
                            ..IpamConfig::default()
                        },
                        IpamConfig {
                            subnet: Some(format!(
                                "{:x}:{:x}:{:x}:{:x}::/64",
                                crate::common::IPV6_PREFIX[0],
                                crate::common::IPV6_PREFIX[1],
                                crate::common::IPV6_PREFIX[2],
                                ipv6
                            )),
                            ..IpamConfig::default()
                        },
                    ]),
                    ..Ipam::default()
                }),
                ..NetworkCreateRequest::default()
            })
            .await
            .inspect_err(|err| error!("Failed to create network: {:?}", err))
            .map_err(|_err| <E as FromDockerFailure>::from())?;

        Ok(Self { id, ipv4, ipv6 })
    }

    pub async fn resolve<E: From<sqlx::Error>>(
        conn: impl Executor<'_, Database = MySql>,
        id: u32,
    ) -> Result<Option<Self>, E> {
        query_as!(Self, "SELECT * FROM `networks` WHERE `id` = ? LIMIT 1", id)
            .fetch_optional(conn)
            .await
            .inspect_err(|err| error!("Failed to resolve network: {:?}", err))
            .map_err(<E as From<sqlx::Error>>::from)
    }

    pub async fn delete<E: From<sqlx::Error> + FromDockerFailure>(
        &self,
        docker: &Docker,
        tx: &mut Transaction<'_, MySql>,
    ) -> Result<(), E> {
        query!("DELETE FROM `networks` WHERE `id` = ?", self.id)
            .execute(&mut **tx)
            .await
            .inspect_err(|err| error!("Failed to delete network: {:?}", err))?;

        match docker
            .inspect_network(
                &format!("{}{}", crate::common::NETWORK_PREFIX, self.id),
                None::<InspectNetworkOptions>,
            )
            .await
        {
            Ok(inspect) => {
                if let Some(containers) = inspect.containers {
                    for container in containers.into_keys() {
                        let _ = docker
                            .disconnect_network(
                                &format!("{}{}", crate::common::NETWORK_PREFIX, self.id),
                                NetworkDisconnectRequest {
                                    container: Some(container),
                                    force: Some(true),
                                },
                            )
                            .await;
                    }
                }
            }
            Err(bollard::errors::Error::DockerResponseServerError {
                status_code: 404, ..
            }) => {}
            Err(err) => {
                error!("Failed to inspect network (delete): {:?}", err);
                return Err(<E as FromDockerFailure>::from());
            }
        }

        match docker
            .remove_network(&format!("{}{}", crate::common::NETWORK_PREFIX, self.id))
            .await
        {
            Ok(())
            | Err(bollard::errors::Error::DockerResponseServerError {
                status_code: 404, ..
            }) => {}
            Err(err) => {
                error!("Failed to delete network: {:?}", err);
                return Err(<E as FromDockerFailure>::from());
            }
        }

        Ok(())
    }
}
