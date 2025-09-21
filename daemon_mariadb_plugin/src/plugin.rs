// SPDX-License-Identifier: AGPL-3.0-only
pub mod proxy;

use crate::plugin::proxy::authenticate_proxy_mode;
use crate::types::{AuthStatus, PluginVIO, PluginVIOInfoProtocol, ServerAuthInfo};
use bollard::Docker;

use bollard::query_parameters::InspectNetworkOptions;
use core::error::Error;
use core::ffi::c_uchar;
use core::mem::{ManuallyDrop, MaybeUninit, size_of};
use core::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use core::str::FromStr as _;
use libc::{SO_PEERCRED, socklen_t, ucred};
use std::ffi::CString;
use std::io;
use std::net::TcpStream;
use std::os::fd::{FromRawFd as _, IntoRawFd as _};
use std::sync::LazyLock;
use tokio::runtime::Runtime;

static DOCKER: LazyLock<Result<Docker, bollard::errors::Error>> =
    LazyLock::new(Docker::connect_with_unix_defaults);
static RUNTIME: LazyLock<Result<Runtime, io::Error>> = LazyLock::new(|| {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
});

pub fn init() -> Result<(), i32> {
    DOCKER.as_ref().map_err(|_err| -1i32)?;
    RUNTIME.as_ref().map_err(|_err| -1i32)?;
    Ok(())
}

#[expect(clippy::unnecessary_wraps, reason = "mariadb needs an ok or errorcode")]
pub const fn deinit() -> Result<(), i32> {
    Ok(())
}

pub fn authenticate(vio: &mut PluginVIO, info: &mut ServerAuthInfo) -> AuthStatus {
    let mut pkt: MaybeUninit<*mut c_uchar> = MaybeUninit::uninit();
    if info.user_name.is_null() && vio.read_packet(&mut pkt) < 0i32 {
        return AuthStatus::Error;
    }

    authenticate_proxy_mode(vio, info)
}

pub fn identify(vio: &mut PluginVIO) -> Result<u32, Box<dyn Error>> {
    let mut vio_info = MaybeUninit::uninit();
    vio.info(&mut vio_info);
    // SAFETY: Passed by mariadb, so should be safe
    let vio_info = unsafe { vio_info.assume_init() };
    match vio_info.protocol {
        PluginVIOInfoProtocol::MYSQL_VIO_TCP => {
            // SAFETY: Passed by mariadb, so should be safe
            let tcp = ManuallyDrop::new(unsafe { TcpStream::from_raw_fd(vio_info.socket) });
            let addr = match tcp.peer_addr() {
                Ok(addr) => addr,
                Err(err) => return Err(err.into()),
            };
            let _ = ManuallyDrop::<TcpStream>::into_inner(tcp).into_raw_fd();
            RUNTIME.as_ref()?.block_on(identify_tcp(addr))
        }
        PluginVIOInfoProtocol::MYSQL_VIO_SOCKET => {
            let mut cred: MaybeUninit<ucred> = MaybeUninit::uninit();
            let mut cred_len = size_of::<ucred>() as socklen_t;
            // SAFETY: Executing syscall. Should be safe
            unsafe {
                if libc::getsockopt(
                    vio_info.socket,
                    libc::SOL_SOCKET,
                    SO_PEERCRED,
                    cred.as_mut_ptr().cast(),
                    &raw mut cred_len,
                ) < 0
                {
                    return Err(io::Error::other("Unable to read pid").into());
                }
            }
            RUNTIME.as_ref()?.block_on(identify_unix(
                // SAFETY: If the syscall succeeded, it should have initialized the reference
                unsafe { cred.assume_init() },
            ))
        }
        _ => Err(io::Error::other("Unsupported connection protocol").into()),
    }
}

pub fn authenticate_as(info: &mut ServerAuthInfo, username: String, external: bool) -> AuthStatus {
    let cstr = match CString::new(username) {
        Ok(str) => str,
        Err(_err) => return AuthStatus::Error,
    };
    info.authenticated_as
        .copy_from_slice(cstr.as_bytes_with_nul());
    if external {
        info.external_user.copy_from_slice(cstr.as_bytes_with_nul());
    }
    AuthStatus::Ok
}

async fn identify_tcp(peer_addr: SocketAddr) -> Result<u32, Box<dyn Error>> {
    #[cfg(debug_assertions)]
    {
        println!("Connection through TCP: {peer_addr}");
    };

    let mut matching = None;

    let network = DOCKER
        .as_ref()?
        .inspect_network(crate::NETWORK_DEFAULT, None::<InspectNetworkOptions>)
        .await?;
    if let Some(containers) = network.containers {
        for network in containers.values() {
            match peer_addr.ip() {
                IpAddr::V4(addr) => {
                    if let Some(ref ipv4) = network.ipv4_address {
                        let ipv4 = ipv4.split_once('/').map_or(ipv4.as_str(), |(ip, _)| ip);
                        let docker_ipv4 = Ipv4Addr::from_str(ipv4);

                        if docker_ipv4.eq(&Ok(addr)) {
                            matching = Some(network.name.clone().unwrap_or_default());
                            break;
                        }
                    }
                }
                IpAddr::V6(addr) => {
                    if let Some(ref ipv6) = network.ipv6_address {
                        let ipv6 = ipv6.split_once('/').map_or(ipv6.as_str(), |(ip, _)| ip);
                        let docker_ipv6 = Ipv6Addr::from_str(ipv6);

                        if docker_ipv6.eq(&Ok(addr)) {
                            matching = Some(network.name.clone().unwrap_or_default());
                            break;
                        }
                    }
                }
            }
        }
    } else {
        #[cfg(debug_assertions)]
        {
            println!("Docker did not return the containers connected to the network");
        }
    }
    if matching.is_none() {
        let network = DOCKER
            .as_ref()?
            .inspect_network(crate::NETWORK_INTERNET, None::<InspectNetworkOptions>)
            .await?;
        if let Some(containers) = network.containers {
            for network in containers.values() {
                match peer_addr.ip() {
                    IpAddr::V4(addr) => {
                        if let Some(ref ipv4) = network.ipv4_address {
                            let ipv4 = ipv4.split_once('/').map_or(ipv4.as_str(), |(ip, _)| ip);
                            let docker_ipv4 = Ipv4Addr::from_str(ipv4);

                            if docker_ipv4.eq(&Ok(addr)) {
                                matching = Some(network.name.clone().unwrap_or_default());
                                break;
                            }
                        }
                    }
                    IpAddr::V6(addr) => {
                        if let Some(ref ipv6) = network.ipv6_address {
                            let ipv6 = ipv6.split_once('/').map_or(ipv6.as_str(), |(ip, _)| ip);
                            let docker_ipv6 = Ipv6Addr::from_str(ipv6);

                            if docker_ipv6.eq(&Ok(addr)) {
                                matching = Some(network.name.clone().unwrap_or_default());
                                break;
                            }
                        }
                    }
                }
            }
        } else {
            #[cfg(debug_assertions)]
            {
                println!("Docker did not return the containers connected to the network");
            }
        }
    }
    match matching.as_deref() {
        None => Err(io::Error::other("No matching container was found.").into()),
        Some(mut name) => {
            if let Some(name_stripped) = name.strip_prefix('/') {
                name = name_stripped;
            }
            let Some(name) = name.strip_prefix(crate::CONTAINER_PREFIX) else {
                return Err(io::Error::other(
                    "Container name does not start with hardcoded prefix.",
                )
                .into());
            };
            name.parse()
                .map_err(|_err| io::Error::other("Unable to parse server id").into())
        }
    }
}

async fn identify_unix(cred: ucred) -> Result<u32, Box<dyn Error>> {
    let pid = cred.pid;
    #[cfg(debug_assertions)]
    {
        println!("Connection through pid: {pid}");
    };
    daemon_common::utils::identify_container_by_pid(DOCKER.as_ref()?, pid)
        .await
        .map_err(Into::into)
}
