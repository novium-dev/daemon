use std::{
    io,
    net::{IpAddr, Ipv6Addr},
};

use base64::{Engine as _, engine::general_purpose::STANDARD};
use daemon_common::key::DaemonKey;
use defguard_wireguard_rs::{
    InterfaceConfiguration, Kernel, WGApi, WireguardInterfaceApi as _, key::Key, net::IpAddrMask,
    peer::Peer,
};
use ed25519_dalek::SigningKey;
use futures::TryStreamExt as _;
use pkcs8::DecodePrivateKey as _;
use sqlx::{Executor, MySql, query};
use tokio::sync::Mutex;
use tracing::warn;

#[expect(dead_code)]
pub async fn sync(
    db: impl Executor<'_, Database = MySql> + Copy,
    key: &DaemonKey,
    fwmark: Option<u32>,
    mtu: Option<u32>,
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

    let private_key = SigningKey::from_pkcs8_der(key.privkey.secret_der()).inspect_err(|err| {
        warn!("Failed to parse ed25519 deamon key for wireguard (using a legacy key?): {err:?}");
    })?;

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

        let tunnel_ip = <[u8; 16]>::try_from(tunnel.ip)
            .map_err(|_vec| io::Error::new(io::ErrorKind::InvalidData, "address not 128 bit"))?;

        let peers = query!(
            "SELECT * FROM `wireguard_peers` WHERE `tunnel` = ?",
            tunnel.id
        )
        .fetch_all(db)
        .await?
        .into_iter()
        .map(|peer| {
            Ok(Peer {
                public_key: Key::new(TryFrom::try_from(peer.public_key).map_err(|_vec| {
                    io::Error::new(io::ErrorKind::InvalidData, "public key not required size")
                })?),
                preshared_key: Some(Key::new(TryFrom::try_from(peer.preshared_key).map_err(
                    |_vec| {
                        io::Error::new(
                            io::ErrorKind::InvalidData,
                            "preshared key not required size",
                        )
                    },
                )?)),
                allowed_ips: vec![IpAddrMask {
                    address: {
                        let mut addr = [0; 16];
                        addr.copy_from_slice(&tunnel_ip[0..8]);
                        addr.copy_from_slice(peer.ip.get(0..8).ok_or_else(|| {
                            io::Error::new(io::ErrorKind::InvalidData, "peer ip suffix not 64 bit")
                        })?);
                        IpAddr::V6(Ipv6Addr::from_bits(u128::from_le_bytes(addr)))
                    },
                    cidr: 128,
                }],
                ..Default::default()
            })
        })
        .collect::<Result<Vec<_>, io::Error>>()?;

        if data.listen_port != tunnel.port
            || data.fwmark != fwmark
            || data
                .private_key
                .is_some_and(|key| !key.as_slice().eq(private_key.as_bytes()))
            || data.peers.len() != peers.len()
            || peers.iter().any(|peer| {
                let Some(current_peer) = data.peers.get(&peer.public_key) else {
                    return true;
                };

                if !current_peer.allowed_ips.iter().eq(peer.allowed_ips.iter())
                    || current_peer.preshared_key != peer.preshared_key
                    || current_peer.endpoint != peer.endpoint
                {
                    return true;
                }

                false
            })
        {
            let prvkey = STANDARD.encode(private_key.as_bytes());
            tokio::task::spawn_blocking(move || {
                interface.configure_interface(&InterfaceConfiguration {
                    name: interface_name,
                    port: tunnel.port,
                    addresses: vec![IpAddrMask {
                        address: IpAddr::V6(Ipv6Addr::from_bits(u128::from_le_bytes(tunnel_ip))),
                        cidr: 64,
                    }],
                    peers,
                    prvkey,
                    mtu,
                    fwmark,
                })?;

                Ok::<_, anyhow::Error>(interface)
            })
            .await??;
        }
    }

    Ok(())
}
