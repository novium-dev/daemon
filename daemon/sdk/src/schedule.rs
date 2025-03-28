use crate::{DaemonSDK, DaemonSDKError, DaemonSDKExtra, RequestBody};
use chrono_tz::Tz;
use cron::Schedule;
use daemon_common::payload::IdResponse;
use daemon_common::payload::schedule::{SchedulePayload, ScheduleRunPolicy, ScheduleStep};
use http::Method;

impl<T: DaemonSDKExtra> DaemonSDK<T> {
    pub async fn create_schedule(
        &self,
        server: u32,
        policy: ScheduleRunPolicy,
        cron: Schedule,
        timezone: Tz,
        steps: Vec<ScheduleStep>,
    ) -> Result<u32, DaemonSDKError> {
        let response = self
            .wrapper(
                Method::PUT,
                format!("schedule/{}", server),
                RequestBody::Json(SchedulePayload {
                    policy,
                    schedule: cron,
                    timezone,
                    steps,
                }),
                None::<&()>,
            )
            .await?;
        if !response.status().is_success() {
            return Err(DaemonSDKError::StatusCode(response.status()));
        }
        response
            .json::<IdResponse>()
            .await
            .map(|response| response.id)
            .map_err(DaemonSDKError::Reqwest)
    }

    pub async fn update_schedule(
        &self,
        server: u32,
        schedule: u32,
        policy: ScheduleRunPolicy,
        cron: Schedule,
        timezone: Tz,
        steps: Vec<ScheduleStep>,
    ) -> Result<(), DaemonSDKError> {
        let response = self
            .wrapper(
                Method::PATCH,
                format!("schedule/{}/{}", server, schedule),
                RequestBody::Json(SchedulePayload {
                    policy,
                    schedule: cron,
                    timezone,
                    steps,
                }),
                None::<&()>,
            )
            .await?;
        if !response.status().is_success() {
            return Err(DaemonSDKError::StatusCode(response.status()));
        }
        Ok(())
    }

    pub async fn delete_schedule(&self, server: u32, schedule: u32) -> Result<(), DaemonSDKError> {
        let response = self
            .wrapper(
                Method::DELETE,
                format!("schedule/{}/{}", server, schedule),
                RequestBody::None::<&()>,
                None::<&()>,
            )
            .await?;
        if !response.status().is_success() {
            return Err(DaemonSDKError::StatusCode(response.status()));
        }
        Ok(())
    }
}
