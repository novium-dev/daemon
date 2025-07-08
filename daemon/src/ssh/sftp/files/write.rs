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
use crate::ssh::sftp::FileHandle;
use russh_sftp::protocol::StatusCode;
use std::io;
use std::io::SeekFrom;
use tokio::io::{AsyncSeekExt as _, AsyncWriteExt as _};
use tracing::{debug, error};

pub async fn write(handle: &mut FileHandle, offset: u64, data: Vec<u8>) -> Result<(), StatusCode> {
    match handle {
        FileHandle::RegularFile(file, _) => write_regular(file, offset, data).await,
        _ => {
            debug!("write on a not regular file handle: {:?}", handle);
            return Err(StatusCode::OpUnsupported);
        }
    }
    .inspect_err(|err| error!("Failed to write file: {:?}", err))
    .map_err(|_err| StatusCode::Failure)
}

#[inline(always)]
async fn write_regular(
    file: &mut tokio::fs::File,
    offset: u64,
    data: Vec<u8>,
) -> Result<(), io::Error> {
    file.seek(SeekFrom::Start(offset)).await?;
    file.write_all(&data).await?;
    Ok(())
}
