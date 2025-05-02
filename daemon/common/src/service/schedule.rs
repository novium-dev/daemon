use super::*;
use crate::payload::schedule::SchedulePayload;
use daemon_framework::service;

define_path_variable!(schedule_id "The schedule id");
define_error!(schedule {
    not_found ("The schedule does not exist" NOT_FOUND),
});

#[service(path = "/schedule/{server_id}", tag = "schedule")]
pub trait ScheduleService {
    #[route("", method = PUT)]
    #[error(server(not_found) sql(failure [sql:*]))]
    #[example("examples/schedule.json")]
    #[doc = "Creates a new schedule"]
    async fn create_schedule(&self, server_id: u32, payload: SchedulePayload) -> Result<u32, _>;

    #[route("/{schedule_id}", method = PATCH)]
    #[error(server(not_found) schedule(not_found) sql(failure [sql:*]))]
    #[example("examples/schedule.json")]
    #[doc = "Updates a schedule"]
    async fn update_schedule(
        &self,
        server_id: u32,
        schedule_id: u32,
        payload: SchedulePayload,
    ) -> Result<(), _>;

    #[route("/{schedule_id}", method = DELETE)]
    #[error(server(not_found) sql(failure [sql:*]))]
    #[doc = "Deletes a schedule"]
    async fn delete_schedule(&self, server_id: u32, schedule_id: u32) -> Result<(), _>;
}
