use crate::Daemon;
use utoipa_axum::router::OpenApiRouter;

#[cfg(feature = "trace_sync")]
pub(crate) mod trace;

#[deprecated]
pub(crate) mod user;

#[deprecated]
pub(crate) fn config_unauthorized() -> OpenApiRouter<Daemon> {
    #[allow(unused_mut)]
    let mut router = OpenApiRouter::new();
    #[cfg(feature = "trace_sync")]
    {
        router = router.merge(trace::config());
    }
    router
}
