pub mod proxy;

use crate::plugin::proxy::authenticate_proxy_mode;
use crate::types::{AuthStatus, PluginVIO, PluginVIOInfoProtocol, ServerAuthInfo};
use bollard::Docker;

use libc::{socklen_t, ucred, SO_PEERCRED};
use std::error::Error;
use std::ffi::{c_char, c_uchar, c_void, CString};
use std::io;
use std::io::ErrorKind;
use std::mem::{size_of, MaybeUninit};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, TcpStream};
use std::os::fd::{FromRawFd, IntoRawFd};
use std::str::FromStr;
use std::sync::RwLock;
use tokio::runtime::Runtime;

static DOCKER: tokio::sync::RwLock<Option<Docker>> = tokio::sync::RwLock::const_new(None);
static RUNTIME: RwLock<Option<Runtime>> = RwLock::new(None);

pub fn init() -> Result<(), i32> {
    *DOCKER.blocking_write() = Some(Docker::connect_with_unix_defaults().map_err(|_err| -1)?);
    *RUNTIME.write().unwrap() = Some(
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .map_err(|_err| -1)?,
    );
    Ok(())
}

pub fn deinit() -> Result<(), i32> {
    *DOCKER.blocking_write() = None;
    *RUNTIME.write().unwrap() = None;
    Ok(())
}

pub fn authenticate(vio: &mut PluginVIO, info: &mut ServerAuthInfo) -> AuthStatus {
    let mut pkt: MaybeUninit<*mut c_uchar> = MaybeUninit::uninit();
    if info.user_name.is_null() && vio.read_packet(&mut pkt) < 0 {
        return AuthStatus::Error;
    }

    authenticate_proxy_mode(vio, info)
}

pub fn identify(vio: &mut PluginVIO) -> Result<u32, Box<dyn Error>> {
    let mut vio_info = MaybeUninit::uninit();
    vio.info(&mut vio_info);
    let vio_info = unsafe { vio_info.assume_init() };
    match vio_info.protocol {
        PluginVIOInfoProtocol::MYSQL_VIO_TCP => {
            let tcp = unsafe { TcpStream::from_raw_fd(vio_info.socket) };
            let addr = match tcp.peer_addr() {
                Ok(addr) => addr,
                Err(err) => return Err(err.into()),
            };
            let _ = tcp.into_raw_fd();
            RUNTIME
                .read()
                .unwrap()
                .as_ref()
                .unwrap()
                .block_on(identify_tcp(addr))
        }
        PluginVIOInfoProtocol::MYSQL_VIO_SOCKET => {
            let mut cred: MaybeUninit<ucred> = MaybeUninit::uninit();
            let mut cred_len = size_of::<ucred>() as socklen_t;
            unsafe {
                if libc::getsockopt(
                    vio_info.socket,
                    libc::SOL_SOCKET,
                    SO_PEERCRED,
                    cred.as_mut_ptr() as *mut c_void,
                    &mut cred_len,
                ) < 0
                {
                    return Err(io::Error::new(ErrorKind::Other, "Unable to read pid").into());
                }

                RUNTIME
                    .read()
                    .unwrap()
                    .as_ref()
                    .unwrap()
                    .block_on(identify_unix(cred.assume_init()))
            }
        }
        _ => Err(io::Error::new(ErrorKind::Other, "Unsupported connection protocol").into()),
    }
}

pub(crate) fn authenticate_as(
    info: &mut ServerAuthInfo,
    username: String,
    external: bool,
) -> AuthStatus {
    let cstr = match CString::new(username) {
        Ok(str) => str,
        Err(_err) => return AuthStatus::Error,
    };
    for (pos, byte) in cstr.as_bytes_with_nul().iter().enumerate() {
        info.authenticated_as[pos] = *byte as c_char;
        if external {
            info.external_user[pos] = *byte as c_char;
        }
    }
    AuthStatus::Ok
}

async fn identify_tcp(peer_addr: SocketAddr) -> Result<u32, Box<dyn Error>> {
    #[cfg(debug_assertions)]
    {
        println!("Connection through TCP: {}", peer_addr);
    }

    let mut matching = None;

    let network = DOCKER
        .read()
        .await
        .as_ref()
        .unwrap()
        .inspect_network::<String>(crate::NETWORK_DEFAULT, None)
        .await?;
    if let Some(containers) = network.containers {
        for network in containers.values() {
            match peer_addr.ip() {
                IpAddr::V4(addr) => {
                    if let Some(ref ipv4) = network.ipv4_address {
                        let ipv4 = ipv4.split_once("/").map(|(ip, _)| ip).unwrap_or(ipv4);
                        let docker_ipv4 = Ipv4Addr::from_str(ipv4);

                        if docker_ipv4.eq(&Ok(addr)) {
                            matching = Some(network.name.clone().unwrap_or_default());
                            break;
                        }
                    }
                }
                IpAddr::V6(addr) => {
                    if let Some(ref ipv6) = network.ipv6_address {
                        let ipv6 = ipv6.split_once("/").map(|(ip, _)| ip).unwrap_or(ipv6);
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
            .read()
            .await
            .as_ref()
            .unwrap()
            .inspect_network::<String>(crate::NETWORK_INTERNET, None)
            .await?;
        if let Some(containers) = network.containers {
            for network in containers.values() {
                match peer_addr.ip() {
                    IpAddr::V4(addr) => {
                        if let Some(ref ipv4) = network.ipv4_address {
                            let ipv4 = ipv4.split_once("/").map(|(ip, _)| ip).unwrap_or(ipv4);
                            let docker_ipv4 = Ipv4Addr::from_str(ipv4);

                            if docker_ipv4.eq(&Ok(addr)) {
                                matching = Some(network.name.clone().unwrap_or_default());
                                break;
                            }
                        }
                    }
                    IpAddr::V6(addr) => {
                        if let Some(ref ipv6) = network.ipv6_address {
                            let ipv6 = ipv6.split_once("/").map(|(ip, _)| ip).unwrap_or(ipv6);
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
    match matching {
        None => Err(io::Error::new(ErrorKind::Other, "No matching container was found.").into()),
        Some(name) => {
            let mut name: &str = &name;
            if name.starts_with("/") {
                name = &name[1..];
            }
            if !name.starts_with(crate::CONTAINER_PREFIX) {
                return Err(io::Error::new(
                    ErrorKind::Other,
                    "Container name does not start with hardcoded prefix.",
                )
                .into());
            }
            let str = &name[crate::CONTAINER_PREFIX.len()..];
            str.parse().map_err(|_err| {
                io::Error::new(ErrorKind::Other, "Unable to parse server id").into()
            })
        }
    }
}

#[allow(static_mut_refs)]
async unsafe fn identify_unix(cred: ucred) -> Result<u32, Box<dyn Error>> {
    let pid = cred.pid;
    #[cfg(debug_assertions)]
    {
        println!("Connection through pid: {}", pid);
    }
    daemon_common::utils::identify_container_by_pid(DOCKER.read().await.as_ref().unwrap(), pid)
        .await
        .map_err(|err| err.into())
}
