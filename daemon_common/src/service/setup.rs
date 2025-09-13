/*
 * Copyright (C) 2025 Markus Probst
 * This program is free software: you can redistribute it and/or modify
 * it under the terms of the GNU Affero General Public License as published by
 * the Free Software Foundation, version 3.
 *
 * This program is distributed in the hope that it will be useful,
 * but WITHOUT ANY WARRANTY; without even the implied warranty of
 * MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
 * GNU Affero General Public License for more details.
 *
 * You should have received a copy of the GNU Affero General Public License
 * along with this program. If not, see <https://www.gnu.org/licenses/>.
 */
use super::*;
use daemon_framework::service;

#[service(tag = "setup")]
pub trait SetupService {
    /// Tests the connection
    #[route("/setup", method = GET)]
    #[error({ AwaitingSetup "This error is expected while in the setup wizard" FORBIDDEN })]
    async fn connection_test(&self) -> Result<(), _>;
}
