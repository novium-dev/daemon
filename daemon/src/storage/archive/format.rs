use axum::http::HeaderValue;

// SPDX-License-Identifier: AGPL-3.0-only
use crate::storage::archive::compression::{ArchiveCompression, CompressionLevel};
use std::io;
use std::io::{Read, Seek, SeekFrom};

/// The container format of an archive, together with the compression wrapping it
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum ArchiveFormat {
    /// A (possibly compressed) tar archive
    Tar(ArchiveCompression),
    /// A zip archive, compression is handled per entry by the zip container
    Zip,
}

impl ArchiveFormat {
    #[rustfmt::skip]
    pub(crate) const fn content_type(&self) -> HeaderValue {
        match self {
            Self::Zip => HeaderValue::from_static("application/zip"),
            Self::Tar(ArchiveCompression::None) => HeaderValue::from_static("application/x-tar"),
            Self::Tar(ArchiveCompression::Zstd { .. }) => HeaderValue::from_static("application/zstd"),
            Self::Tar(ArchiveCompression::Xz { .. }) => HeaderValue::from_static("application/x-xz"),
            Self::Tar(ArchiveCompression::Zlib { .. }) => HeaderValue::from_static("application/zlib"),
            Self::Tar(ArchiveCompression::Lz4 { .. }) => HeaderValue::from_static("application/x-lz4"),
            Self::Tar(ArchiveCompression::Gzip { .. }) => HeaderValue::from_static("application/gzip"),
            Self::Tar(ArchiveCompression::Bzip2 { .. }) => HeaderValue::from_static("application/x-bzip2"),
        }
    }

    /// If successful, returns file handle with seek 0
    pub(crate) fn detect_blocking<'a, R: Read + Seek + Send + Sync + 'a>(
        mut read: R,
    ) -> Result<Option<Self>, io::Error> {
        const TAR_MAGIC_OFFSET: u64 = 257;

        let mut buf = [0u8; 8];
        read.seek(SeekFrom::Start(0))?;
        read.read_exact(&mut buf)?;

        // Sources: https://en.wikipedia.org/wiki/List_of_file_signatures
        let format = match buf {
            [0x50, 0x4B, 0x03, 0x04, ..]
            | [0x50, 0x4B, 0x05, 0x06, ..]
            | [0x50, 0x4B, 0x07, 0x08, ..] => Self::Zip,
            [0x28, 0xB5, 0x2F, 0xFD, ..] => Self::Tar(ArchiveCompression::Zstd {
                level: CompressionLevel::Default,
            }),
            [0xFD, 0x37, 0x7A, 0x58, 0x5A, 0x00, ..] => Self::Tar(ArchiveCompression::Xz {
                level: CompressionLevel::Default,
            }),
            [
                0x78,
                0x01 | 0x5E | 0x9C | 0xDA | 0x20 | 0x7D | 0xBB | 0xF9,
                ..,
            ] => Self::Tar(ArchiveCompression::Zlib {
                level: CompressionLevel::Default,
            }),
            [0x04, 0x22, 0x4D, 0x18, ..] => Self::Tar(ArchiveCompression::Lz4 {
                level: CompressionLevel::Default,
            }),
            [0x1F, 0x8B, ..] => Self::Tar(ArchiveCompression::Gzip {
                level: CompressionLevel::Default,
            }),
            [0x42, 0x5A, 0x68, ..] => Self::Tar(ArchiveCompression::Bzip2 {
                level: CompressionLevel::Default,
            }),
            [0x75, 0x73, 0x74, 0x61, 0x72, 0x00, 0x30, 0x30]
            | [0x75, 0x73, 0x74, 0x61, 0x72, 0x20, 0x20, 0x00] => {
                Self::Tar(ArchiveCompression::None)
            }
            _ => {
                read.seek(SeekFrom::Start(0))?;
                return Ok(None);
            }
        };
        if let Self::Tar(compression) = format
            && compression != ArchiveCompression::None
        {
            read.seek(SeekFrom::Start(0))?;
            let mut decoder = compression.create_decoder(&mut read)?;
            decoder.read_exact(&mut [0; TAR_MAGIC_OFFSET as usize])?;
            decoder.read_exact(&mut buf)?;
            drop(decoder);
            if buf != [0x75, 0x73, 0x74, 0x61, 0x72, 0x00, 0x30, 0x30]
                && buf != [0x75, 0x73, 0x74, 0x61, 0x72, 0x20, 0x20, 0x00]
            {
                return Ok(None);
            }
        }
        read.seek(SeekFrom::Start(0))?;
        Ok(Some(format))
    }
}
