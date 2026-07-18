use std::{
    io,
    net::{IpAddr, Ipv6Addr},
};

use defguard_wireguard_rs::{
    InterfaceConfiguration, Kernel, WGApi, WireguardInterfaceApi as _, net::IpAddrMask,
};
use futures::TryStreamExt as _;
use sqlx::{Executor, MySql, query};
use tokio::sync::Mutex;

#[expect(dead_code)]
pub async fn sync(
    db: impl Executor<'_, Database = MySql> + Copy,
    interface_lock: &Mutex<Option<rtnetlink::Handle>>,
) -> Result<(), anyhow::Error> {
    let mut tunnels = query!("SELECT `id`, `ip`, `port` FROM `wireguard_tunnels`").fetch(db);

    let mut lock = interface_lock.lock().await;

    let handle = if let Some(handle) = &mut *lock {
        handle
    } else {
        let (connection, handle, _) = rtnetlink::new_connection()?;
        tokio::task::spawn(connection);
        lock.insert(handle)
    };

    while let Some(tunnel) = tunnels.try_next().await? {
        let interface_name = format!("wg_daemon_{}", tunnel.id);

        let exists = handle
            .link()
            .get()
            .match_name(interface_name.clone())
            .execute()
            .try_next()
            .await?
            .is_some();

        let interface_name_clone = interface_name.clone();
        let (interface, data) = tokio::task::spawn_blocking(move || {
            let mut interface: WGApi<Kernel> = WGApi::new(interface_name_clone)?;

            if !exists {
                interface.create_interface()?;
            }

            let data = interface.read_interface_data()?;

            Ok::<_, anyhow::Error>((interface, data))
        })
        .await??;

        let peers = Vec::new(); // TODO

        // TODO: Compare fully
        if data.listen_port != tunnel.port {
            tokio::task::spawn_blocking(move || {
                interface.configure_interface(&InterfaceConfiguration {
                    name: interface_name,
                    port: tunnel.port,
                    addresses: vec![IpAddrMask {
                        #[expect(clippy::little_endian_bytes)]
                        address: IpAddr::V6(Ipv6Addr::from_bits(u128::from_le_bytes(
                            <[u8; 16]>::try_from(tunnel.ip).map_err(|_vec| {
                                io::Error::new(io::ErrorKind::InvalidData, "address not 128 bit")
                            })?,
                        ))),
                        cidr: 64,
                    }],
                    peers,
                    prvkey: String::new(), // TODO
                    mtu: None,
                    fwmark: None,
                })?;

                Ok::<_, anyhow::Error>(interface)
            })
            .await??;
        }
    }

    Ok(())
}
