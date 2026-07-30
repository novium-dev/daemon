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
    pub(crate) fn detect_blocking<'a, R: Read + Seek + Send + Sync + 'a>(
        mut read: R,
    ) -> Result<Option<Self>, io::Error> {
        const TAR_MAGIC_OFFSET: u64 = 257;

        let mut buf = [0u8; 8];
        read.seek(SeekFrom::Start(0))?;
        read.read_exact(&mut buf)?;

        // Sources: https://en.wikipedia.org/wiki/List_of_file_signatures
        if matches!(
            buf,
            [0x50, 0x4B, 0x03, 0x04, ..]
                | [0x50, 0x4B, 0x05, 0x06, ..]
                | [0x50, 0x4B, 0x07, 0x08, ..]
        ) {
            read.seek(SeekFrom::Start(0))?;
            return Ok(Some(Self::Zip));
        }

        let compression = match buf {
            [0x28, 0xB5, 0x2F, 0xFD, _, _, _, _] => ArchiveCompression::Zstd {
                level: CompressionLevel::Default,
            },
            [0xFD, 0x37, 0x7A, 0x58, 0x5A, 0x00, _, _] => ArchiveCompression::Xz {
                level: CompressionLevel::Default,
            },
            [
                0x78,
                0x01 | 0x5E | 0x9C | 0xDA | 0x20 | 0x7D | 0xBB | 0xF9,
                _,
                _,
                _,
                _,
                _,
                _,
            ] => ArchiveCompression::Zlib {
                level: CompressionLevel::Default,
            },
            [0x04, 0x22, 0x4D, 0x18, _, _, _, _] => ArchiveCompression::Lz4 {
                level: CompressionLevel::Default,
            },
            [0x1F, 0x8B, _, _, _, _, _, _] => ArchiveCompression::Gzip {
                level: CompressionLevel::Default,
            },
            [0x42, 0x5A, 0x68, _, _, _, _, _] => ArchiveCompression::Bzip2 {
                level: CompressionLevel::Default,
            },
            _ => ArchiveCompression::None,
        };
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
        read.seek(SeekFrom::Start(0))?;
        Ok(Some(Self::Tar(compression)))
    }
}
