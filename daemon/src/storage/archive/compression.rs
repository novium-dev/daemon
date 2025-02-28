use serde::de::{Error, Visitor};
use serde::{Deserialize, Serialize, Serializer};
use std::fmt::Formatter;
use std::io;
use std::io::{ErrorKind, Read, Write};
use tracing::error;

#[derive(Serialize, Deserialize, Copy, Clone, Debug)]
#[serde(tag = "type")]
pub(crate) enum ArchiveCompression {
    #[serde(rename = "none")]
    None,
    #[serde(rename = "zstd")]
    Zstd {
        #[serde(deserialize_with = "deserialize_compression_level")]
        #[serde(serialize_with = "serialize_compression_level")]
        #[serde(default = "default_level")]
        level: CompressionLevel,
    },
    /*#[serde(rename = "xz")]
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
        level: async_compression::Level,
    },
    #[serde(rename = "gzip")]
    Gzip {
        #[serde(deserialize_with = "deserialize_compression_level")]
        #[serde(serialize_with = "serialize_compression_level")]
        #[serde(default = "default_level")]
        level: async_compression::Level,
    },
    #[serde(rename = "lzma")]
    Lzma {
        #[serde(deserialize_with = "deserialize_compression_level")]
        #[serde(serialize_with = "serialize_compression_level")]
        #[serde(default = "default_level")]
        level: async_compression::Level,
    },
    #[serde(rename = "brotli")]
    Brotli {
        #[serde(deserialize_with = "deserialize_compression_level")]
        #[serde(serialize_with = "serialize_compression_level")]
        #[serde(default = "default_level")]
        level: async_compression::Level,
    },
    #[serde(rename = "bzip2")]
    Bzip2 {
        #[serde(deserialize_with = "deserialize_compression_level")]
        #[serde(serialize_with = "serialize_compression_level")]
        #[serde(default = "default_level")]
        level: crate::storage::archive::compression::prelude::Level,
    },*/
}

impl ArchiveCompression {
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
        }
    }

    pub(crate) fn create_decoder<R: Read + Send + Sync + 'static>(
        &self,
        read: R,
    ) -> Result<Box<dyn Read + Send + Sync + 'static>, io::Error> {
        Ok(match self {
            ArchiveCompression::None => Box::new(read),
            ArchiveCompression::Zstd { .. } => Box::new(zstd::Decoder::new(read)?),
        })
    }

    pub(crate) fn create_encoder<W: Write + Send + Sync + 'static>(
        &self,
        write: W,
    ) -> Result<Box<dyn Write + Send + Sync + 'static>, io::Error> {
        if !self.valid_level() {
            error!("Invalid level in compression tried to use: {:?}", self);
            return Err(io::Error::new(
                ErrorKind::Other,
                "Invalid compression level",
            ));
        }
        Ok(match self {
            ArchiveCompression::None => Box::new(write),
            ArchiveCompression::Zstd { level } => Box::new(zstd::Encoder::new(
                write,
                match level {
                    CompressionLevel::Fastest => *zstd::compression_level_range().start(),
                    CompressionLevel::Best => *zstd::compression_level_range().end(),
                    CompressionLevel::Default => zstd::DEFAULT_COMPRESSION_LEVEL,
                    CompressionLevel::Precise(level) => *level,
                },
            )?),
        })
    }

    pub(crate) fn ext(&self) -> &'static str {
        match self {
            ArchiveCompression::None => "",
            ArchiveCompression::Zstd { .. } => ".zst",
            /*ArchiveCompression::Xz { .. } => ".xz",
            ArchiveCompression::Zlib { .. } => ".zz",
            ArchiveCompression::Gzip { .. } => ".gz",
            ArchiveCompression::Lzma { .. } => ".lzma",
            ArchiveCompression::Brotli { .. } => ".br",
            ArchiveCompression::Bzip2 { .. } => ".bz2",*/
        }
    }
}

#[derive(Debug, Copy, Clone)]
pub enum CompressionLevel {
    Fastest,
    Best,
    Default,
    Precise(i32),
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
