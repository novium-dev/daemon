use defguard_wireguard_rs::{Kernel, WGApi, WireguardInterfaceApi as _};
use futures::TryStreamExt;
use sqlx::{Executor, MySql, query};
use tokio::sync::Mutex;

pub async fn sync(
    db: impl Executor<'_, Database = MySql> + Copy,
    interface_lock: &Mutex<Option<rtnetlink::Handle>>,
) -> Result<(), anyhow::Error> {
    let mut tunnels = query!("SELECT `id`, `ip`, `port` FROM `wireguard_tunnels`").fetch(db);

    let mut lock = interface_lock.lock().await;

    // TODO: Create new rtlink connection and spawn worker, if none.
    let handle = lock.as_mut().unwrap();

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

        let (interface, data) = tokio::task::spawn_blocking(move || {
            let mut interface: WGApi<Kernel> = WGApi::new(interface_name)?;

            if !exists {
                interface.create_interface()?;
            }

            let data = interface.read_interface_data()?;

            Ok::<_, anyhow::Error>((interface, data))
        })
        .await??;

        // TODO: Compare and configure
    }

    Ok(())
}
