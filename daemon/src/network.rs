use crate::web::sql_error_to_http;
use axum::http::StatusCode;
use bollard::models::{Ipam, IpamConfig};
use bollard::network::{CreateNetworkOptions, DisconnectNetworkOptions};
use bollard::Docker;
use sqlx::{query, query_as, Error, Executor, MySql, Transaction};

#[derive(Debug)]
pub(crate) enum NetworkError {
    Sql(sqlx::Error),
    #[allow(dead_code)]
    Docker(bollard::errors::Error),
}

impl From<sqlx::Error> for NetworkError {
    fn from(value: Error) -> Self {
        NetworkError::Sql(value)
    }
}

impl NetworkError {
    pub(crate) fn into_http(self) -> StatusCode {
        match self {
            NetworkError::Sql(err) => sql_error_to_http(err),
            NetworkError::Docker(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

#[derive(Debug, Clone, Eq, PartialEq, sqlx::FromRow)]
pub(crate) struct Network {
    pub(crate) id: u32,
    pub(crate) ipv4: u16,
    pub(crate) ipv6: u16,
}

impl Network {
    pub(crate) async fn create(
        docker: &Docker,
        tx: &mut Transaction<'_, MySql>,
        ipv4: u16,
        ipv6: u16,
    ) -> Result<Network, NetworkError> {
        let id = query!(
            "INSERT INTO `networks` (`ipv4`, `ipv6`) VALUES (?, ?)",
            ipv4,
            ipv6
        )
        .execute(&mut **tx)
        .await
        .map_err(NetworkError::Sql)?
        .last_insert_id() as u32;

        let ipv4_split = [(ipv4 >> 8) as u8, ipv4 as u8];

        docker
            .create_network(CreateNetworkOptions {
                name: format!("{}{}", crate::common::NETWORK_PREFIX, id),
                enable_ipv6: true,
                internal: true,
                ipam: Ipam {
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
                },
                ..CreateNetworkOptions::default()
            })
            .await
            .map_err(NetworkError::Docker)?;

        Ok(Network { id, ipv4, ipv6 })
    }

    pub(crate) async fn resolve(
        conn: impl Executor<'_, Database = MySql>,
        id: u32,
    ) -> Result<Option<Network>, NetworkError> {
        query_as!(
            Network,
            "SELECT * FROM `networks` WHERE `id` = ? LIMIT 1",
            id
        )
        .fetch_optional(conn)
        .await
        .map_err(NetworkError::Sql)
    }

    pub(crate) async fn delete(
        &self,
        docker: &Docker,
        tx: &mut Transaction<'_, MySql>,
    ) -> Result<(), NetworkError> {
        query!("DELETE FROM `networks` WHERE `id` = ?", self.id)
            .execute(&mut **tx)
            .await
            .map_err(NetworkError::Sql)?;

        match docker
            .inspect_network::<String>(
                &format!("{}{}", crate::common::NETWORK_PREFIX, self.id),
                None,
            )
            .await
        {
            Ok(inspect) => {
                if let Some(containers) = inspect.containers {
                    for container in containers.into_keys() {
                        let _ = docker
                            .disconnect_network(
                                &format!("{}{}", crate::common::NETWORK_PREFIX, self.id),
                                DisconnectNetworkOptions {
                                    container,
                                    force: true,
                                },
                            )
                            .await;
                    }
                }
            }
            Err(bollard::errors::Error::DockerResponseServerError {
                status_code: 404, ..
            }) => {}
            Err(err) => return Err(NetworkError::Docker(err)),
        }

        match docker
            .remove_network(&format!("{}{}", crate::common::NETWORK_PREFIX, self.id))
            .await
        {
            Ok(_) => {}
            Err(bollard::errors::Error::DockerResponseServerError {
                status_code: 404, ..
            }) => {}
            Err(err) => return Err(NetworkError::Docker(err)),
        }

        Ok(())
    }
}
