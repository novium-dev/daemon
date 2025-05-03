use bzip2::write::BzEncoder;
use flate2::write::{GzEncoder, ZlibEncoder};
use liblzma::write::XzEncoder;
use serde::de::{Error, Visitor};
use serde::{Deserialize, Serialize, Serializer};
use std::cmp::PartialEq;
use std::fmt::{Arguments, Formatter};
use std::io;
use std::io::{ErrorKind, IoSlice, Read, Seek, SeekFrom, Write};
use tracing::error;

#[derive(Serialize, Deserialize, Copy, Clone, Debug, PartialEq, Eq)]
#[serde(tag = "type")]
pub enum ArchiveCompression {
    #[serde(rename = "none")]
    None,
    #[serde(rename = "zstd")]
    Zstd {
        #[serde(deserialize_with = "deserialize_compression_level")]
        #[serde(serialize_with = "serialize_compression_level")]
        #[serde(default = "default_level")]
        level: CompressionLevel,
    },
    #[serde(rename = "xz")]
    Xz {
        #[serde(deserialize_with = "deserialize_compression_level")]
        #[serde(serialize_with = "serialize_compression_level")]
        #[serde(default = "default_level")]
        level: CompressionLevel,
    },
    #[serde(rename = "zlib")]
    Zlib {
        #[serde(deserialize_with = "deserialize_compression_level")]
        #[serde(serialize_with = "serialize_compression_level")]
        #[serde(default = "default_level")]
        level: CompressionLevel,
    },
    #[serde(rename = "lz4")]
    Lz4 {
        #[serde(deserialize_with = "deserialize_compression_level")]
        #[serde(serialize_with = "serialize_compression_level")]
        #[serde(default = "default_level")]
        level: CompressionLevel,
    },
    #[serde(rename = "gzip")]
    Gzip {
        #[serde(deserialize_with = "deserialize_compression_level")]
        #[serde(serialize_with = "serialize_compression_level")]
        #[serde(default = "default_level")]
        level: CompressionLevel,
    },
    #[serde(rename = "bzip2")]
    Bzip2 {
        #[serde(deserialize_with = "deserialize_compression_level")]
        #[serde(serialize_with = "serialize_compression_level")]
        #[serde(default = "default_level")]
        level: CompressionLevel,
    },
}

impl ArchiveCompression {
    pub(crate) fn detect_blocking<'a, R: Read + Seek + Send + Sync + 'a>(
        mut read: R,
    ) -> Result<Option<Self>, io::Error> {
        const TAR_MAGIC_OFFSET: u64 = 257;

        let mut buf = [0u8; 8];
        read.seek(SeekFrom::Start(0))?;
        read.read_exact(&mut buf)?;

        // Sources: https://en.wikipedia.org/wiki/List_of_file_signatures
        let compression = match buf {
            [0x28, 0xB5, 0x2F, 0xFD, _, _, _, _] => ArchiveCompression::Zstd {
                level: CompressionLevel::Default,
            },
            [0xFD, 0x37, 0x7A, 0x58, 0x5A, 0x00, _, _] => ArchiveCompression::Xz {
                level: CompressionLevel::Default,
            },
            [0x78, 0x01, _, _, _, _, _, _]
            | [0x78, 0x5E, _, _, _, _, _, _]
            | [0x78, 0x9C, _, _, _, _, _, _]
            | [0x78, 0xDA, _, _, _, _, _, _]
            | [0x78, 0x20, _, _, _, _, _, _]
            | [0x78, 0x7D, _, _, _, _, _, _]
            | [0x78, 0xBB, _, _, _, _, _, _]
            | [0x78, 0xF9, _, _, _, _, _, _] => ArchiveCompression::Zlib {
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
        Ok(Some(compression))
    }

    pub(crate) fn valid_level(&self) -> bool {
        match self {
            ArchiveCompression::None => true,
            ArchiveCompression::Zstd {
                level: CompressionLevel::Precise(level),
            } => zstd::compression_level_range().contains(level),
            ArchiveCompression::Zstd {
                level:
                    CompressionLevel::Best | CompressionLevel::Default | CompressionLevel::Fastest,
            } => true,
            ArchiveCompression::Xz {
                level: CompressionLevel::Precise(level),
            } => *level >= 0 && *level <= 9,
            ArchiveCompression::Xz {
                level:
                    CompressionLevel::Best | CompressionLevel::Default | CompressionLevel::Fastest,
            } => true,
            ArchiveCompression::Zlib {
                level: CompressionLevel::Precise(level),
            } => *level >= 0 && *level <= 9,
            ArchiveCompression::Zlib {
                level:
                    CompressionLevel::Best | CompressionLevel::Default | CompressionLevel::Fastest,
            } => true,
            ArchiveCompression::Lz4 {
                level: CompressionLevel::Precise(level),
            } => *level >= 1 && *level <= 12,
            ArchiveCompression::Lz4 {
                level:
                    CompressionLevel::Best | CompressionLevel::Default | CompressionLevel::Fastest,
            } => true,
            ArchiveCompression::Gzip {
                level: CompressionLevel::Precise(level),
            } => *level >= 1 && *level <= 9,
            ArchiveCompression::Gzip {
                level:
                    CompressionLevel::Best | CompressionLevel::Default | CompressionLevel::Fastest,
            } => true,
            ArchiveCompression::Bzip2 { level } => level.bzip2().is_some(),
        }
    }

    pub(crate) fn create_decoder<'a, R: Read + Send + 'a>(
        &self,
        read: R,
    ) -> Result<Box<dyn Read + Send + 'a>, io::Error> {
        Ok(match self {
            ArchiveCompression::None => Box::new(read),
            ArchiveCompression::Zstd { .. } => Box::new(zstd::Decoder::new(read)?),
            ArchiveCompression::Xz { .. } => Box::new(liblzma::read::XzDecoder::new(read)),
            ArchiveCompression::Zlib { .. } => Box::new(flate2::read::ZlibDecoder::new(read)),
            ArchiveCompression::Lz4 { .. } => Box::new(lz4::Decoder::new(read)?),
            ArchiveCompression::Gzip { .. } => Box::new(flate2::read::GzDecoder::new(read)),
            ArchiveCompression::Bzip2 { .. } => Box::new(bzip2::read::BzDecoder::new(read)),
        })
    }

    pub(crate) fn create_encoder<W: Write + Send + 'static>(
        &self,
        write: W,
    ) -> Result<Box<dyn CompressionEncoder<W> + Send + 'static>, io::Error> {
        if !self.valid_level() {
            error!("Invalid level in compression tried to use: {:?}", self);
            return Err(io::Error::new(
                ErrorKind::Other,
                "Invalid compression level",
            ));
        }
        Ok(match self {
            ArchiveCompression::None => Box::new(PassthroughEncoder(write)),
            ArchiveCompression::Zstd { level } => Box::new(zstd::Encoder::new(
                write,
                match level {
                    CompressionLevel::Fastest => *zstd::compression_level_range().start(),
                    CompressionLevel::Best => *zstd::compression_level_range().end(),
                    CompressionLevel::Default => zstd::DEFAULT_COMPRESSION_LEVEL,
                    CompressionLevel::Precise(level) => *level,
                },
            )?),
            ArchiveCompression::Xz { level } => Box::new(liblzma::write::XzEncoder::new(
                write,
                match level {
                    CompressionLevel::Fastest => 0,
                    CompressionLevel::Best => 9,
                    CompressionLevel::Default => 6,
                    CompressionLevel::Precise(level) => *level as u32,
                },
            )),
            ArchiveCompression::Zlib { level } => {
                Box::new(flate2::write::ZlibEncoder::new(write, level.flate2()))
            }
            ArchiveCompression::Lz4 { level } => Box::new(
                lz4::EncoderBuilder::new()
                    .level(match level {
                        CompressionLevel::Fastest => 1,
                        CompressionLevel::Best => 12,
                        CompressionLevel::Default => 1,
                        CompressionLevel::Precise(level) => *level as u32,
                    })
                    .build(write)?,
            ),
            ArchiveCompression::Gzip { level } => {
                Box::new(flate2::write::GzEncoder::new(write, level.flate2()))
            }
            ArchiveCompression::Bzip2 { level } => {
                Box::new(bzip2::write::BzEncoder::new(write, level.bzip2().unwrap()))
            }
        })
    }

    pub(crate) fn ext(&self) -> &'static str {
        match self {
            ArchiveCompression::None => "",
            ArchiveCompression::Zstd { .. } => ".zst",
            ArchiveCompression::Xz { .. } => ".xz",
            ArchiveCompression::Zlib { .. } => ".zz",
            ArchiveCompression::Gzip { .. } => ".gz",
            ArchiveCompression::Bzip2 { .. } => ".bz2",
            ArchiveCompression::Lz4 { .. } => ".lz4",
        }
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum CompressionLevel {
    Fastest,
    Best,
    Default,
    Precise(i32),
}

impl CompressionLevel {
    pub(crate) fn flate2(&self) -> flate2::Compression {
        match self {
            CompressionLevel::Fastest => flate2::Compression::fast(),
            CompressionLevel::Best => flate2::Compression::best(),
            CompressionLevel::Default => flate2::Compression::default(),
            CompressionLevel::Precise(level) => flate2::Compression::new(*level as u32),
        }
    }

    pub(crate) fn bzip2(&self) -> Option<bzip2::Compression> {
        match self {
            CompressionLevel::Fastest => Some(bzip2::Compression::fast()),
            CompressionLevel::Best => Some(bzip2::Compression::best()),
            CompressionLevel::Default => Some(bzip2::Compression::default()),
            CompressionLevel::Precise(level) => bzip2::Compression::try_new(*level as u32),
        }
    }
}

fn default_level() -> CompressionLevel {
    CompressionLevel::Default
}

fn serialize_compression_level<S: Serializer>(
    v: &CompressionLevel,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    match *v {
        CompressionLevel::Fastest => serializer.serialize_str("fastest"),
        CompressionLevel::Best => serializer.serialize_str("default"),
        CompressionLevel::Default => serializer.serialize_str("best"),
        CompressionLevel::Precise(level) => serializer.serialize_i32(level),
    }
}

fn deserialize_compression_level<'de, D>(deserializer: D) -> Result<CompressionLevel, D::Error>
where
    D: serde::Deserializer<'de>,
{
    struct CompressionLevelVisitor;
    impl Visitor<'_> for CompressionLevelVisitor {
        type Value = CompressionLevel;

        fn expecting(&self, formatter: &mut Formatter) -> std::fmt::Result {
            write!(
                formatter,
                "an integer or \"fastest\", \"default\", \"best\""
            )
        }

        fn visit_i64<E>(self, v: i64) -> Result<Self::Value, E>
        where
            E: Error,
        {
            Ok(CompressionLevel::Precise(v as i32))
        }

        fn visit_u64<E>(self, v: u64) -> Result<Self::Value, E>
        where
            E: Error,
        {
            Ok(CompressionLevel::Precise(v as i32))
        }

        fn visit_str<E>(self, v: &str) -> Result<Self::Value, E>
        where
            E: Error,
        {
            match v {
                "fastest" => Ok(CompressionLevel::Fastest),
                "default" => Ok(CompressionLevel::Default),
                "best" => Ok(CompressionLevel::Best),
                _ => Err(E::custom("Invalid compression level")),
            }
        }
    }
    let visitor = CompressionLevelVisitor;
    deserializer.deserialize_any(visitor)
}

pub trait CompressionEncoder<W: Write>: Write {
    fn finish(self: Box<Self>) -> Result<W, io::Error>;
}

pub struct PassthroughEncoder<W: Write>(W);

impl<W: Write> Write for PassthroughEncoder<W> {
    fn write_all(&mut self, buf: &[u8]) -> io::Result<()> {
        self.0.write_all(buf)
    }

    fn write_fmt(&mut self, fmt: Arguments<'_>) -> io::Result<()> {
        self.0.write_fmt(fmt)
    }

    fn write_vectored(&mut self, bufs: &[IoSlice<'_>]) -> io::Result<usize> {
        self.0.write_vectored(bufs)
    }

    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.0.flush()
    }
}

impl<W: Write> CompressionEncoder<W> for PassthroughEncoder<W> {
    fn finish(self: Box<Self>) -> Result<W, io::Error> {
        Ok(self.0)
    }
}

impl<W: Write> CompressionEncoder<W> for zstd::Encoder<'_, W> {
    fn finish(self: Box<Self>) -> Result<W, io::Error> {
        (*self).try_finish().map_err(|(_, err)| err)
    }
}

impl<W: Write> CompressionEncoder<W> for XzEncoder<W> {
    fn finish(self: Box<Self>) -> Result<W, io::Error> {
        (*self).finish()
    }
}

impl<W: Write> CompressionEncoder<W> for ZlibEncoder<W> {
    fn finish(self: Box<Self>) -> Result<W, io::Error> {
        (*self).finish()
    }
}

impl<W: Write> CompressionEncoder<W> for lz4::Encoder<W> {
    fn finish(self: Box<Self>) -> Result<W, io::Error> {
        let (inner, result) = (*self).finish();
        result.map(|_| inner)
    }
}

impl<W: Write> CompressionEncoder<W> for GzEncoder<W> {
    fn finish(self: Box<Self>) -> Result<W, io::Error> {
        (*self).finish()
    }
}

impl<W: Write> CompressionEncoder<W> for BzEncoder<W> {
    fn finish(self: Box<Self>) -> Result<W, io::Error> {
        (*self).finish()
    }
}
