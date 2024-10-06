use crate::storage::StorageIdentifier;
use crate::utils::parse_path;
use axum::body::BodyDataStream;
use axum::http::StatusCode;
use daemon_common::files::FileMode;
use daemon_common::payload::files::FileResponse;
use futures_util::TryStreamExt;
use std::fs::Metadata;
use std::io;
use std::io::ErrorKind;
use std::os::unix::prelude::*;
use std::path::PathBuf;
use tokio::fs::OpenOptions;
use tokio::io::BufWriter;
use tokio_util::io::StreamReader;

pub(crate) async fn list_files(
    storage: StorageIdentifier,
    path: String,
) -> Result<Vec<FileResponse>, StatusCode> {
    let path = path0(storage, path, false).await?;

    let mut read_dir = tokio::fs::read_dir(&path).await.map_err(|err| match err {
        err if err.kind() == ErrorKind::NotFound => StatusCode::NOT_FOUND,
        _ => StatusCode::INTERNAL_SERVER_ERROR,
    })?;

    let mut vec = Vec::new();
    while let Some(entry) = read_dir
        .next_entry()
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
    {
        let metadata = entry
            .metadata()
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        let mut link_path = None;
        if metadata.is_symlink() {
            link_path = Some(
                tokio::fs::read_link(&path)
                    .await
                    .map_err(|_err| StatusCode::INTERNAL_SERVER_ERROR)?
                    .to_string_lossy()
                    .to_string(),
            )
        }
        vec.push(
            metadata.into_fileresponse(entry.file_name().to_string_lossy().to_string(), link_path),
        );
    }

    Ok(vec)
}

pub(crate) async fn stat_file(
    storage: StorageIdentifier,
    path: String,
) -> Result<FileResponse, StatusCode> {
    let path = path0(storage, path, false).await?;

    let metadata = tokio::fs::symlink_metadata(&path)
        .await
        .map_err(|err| match err {
            err if err.kind() == ErrorKind::NotFound => StatusCode::NOT_FOUND,
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        })?;

    let filename = match path.file_name() {
        None => return Err(StatusCode::INTERNAL_SERVER_ERROR),
        Some(filename) => filename.to_string_lossy().to_string(),
    };
    let mut link_path = None;
    if metadata.is_symlink() {
        link_path = Some(
            tokio::fs::read_link(&path)
                .await
                .map_err(|_err| StatusCode::INTERNAL_SERVER_ERROR)?
                .to_string_lossy()
                .to_string(),
        )
    }
    Ok(metadata.into_fileresponse(filename, link_path))
}

pub(crate) async fn rename_file(
    storage: StorageIdentifier,
    from: String,
    to: String,
) -> Result<(), StatusCode> {
    let from = path0(storage, from, false).await?;
    let to = path0(storage, to, true).await?;

    tokio::fs::rename(from, to).await.map_err(|err| match err {
        err if err.kind() == ErrorKind::NotFound => StatusCode::NOT_FOUND,
        _ => StatusCode::INTERNAL_SERVER_ERROR,
    })
}

pub(crate) async fn write_file(
    storage: StorageIdentifier,
    path: String,
    stream: BodyDataStream,
) -> Result<(), StatusCode> {
    let path = path0(storage, path, true).await?;
    let file = OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(&path)
        .await
        .map_err(|_err| StatusCode::INTERNAL_SERVER_ERROR)?;
    let mut reader =
        StreamReader::new(stream.map_err(|err| io::Error::new(io::ErrorKind::Other, err)));
    let mut writer = BufWriter::new(file);
    tokio::io::copy(&mut reader, &mut writer)
        .await
        .map_err(|_err| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(())
}

async fn path0(
    storage: StorageIdentifier,
    mut path: String,
    check_parent: bool,
) -> Result<PathBuf, StatusCode> {
    let (components, ends_with_slash) =
        parse_path(&mut path).map_err(|_| StatusCode::BAD_REQUEST)?;
    let mut components: Vec<String> = components.into_iter().map(String::from).collect();

    if !components.is_empty() && ends_with_slash && !components[components.len() - 1].ends_with('/')
    {
        let index = components.len() - 1;
        components[index] = format!("{}/", components[index]);
    }
    let mut r#final = None;
    if !components.is_empty() && check_parent {
        r#final = Some(components.remove(components.len() - 1));
    }

    let mut base = storage.as_pathbuf();
    let mut path = base.clone();
    base = tokio::fs::canonicalize(base)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    for component in components {
        path = path.join(component);
    }

    path = tokio::fs::canonicalize(path)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    if path.strip_prefix(base).is_err() {
        return Err(StatusCode::BAD_REQUEST);
    }
    if let Some(component) = r#final {
        path = path.join(component);
    }
    Ok(path)
}

pub trait IntoFileResponse {
    fn into_fileresponse(self, filename: String, path: Option<String>) -> FileResponse;
}

impl IntoFileResponse for Metadata {
    fn into_fileresponse(self, filename: String, path: Option<String>) -> FileResponse {
        let mut r#type = 0;
        let file_type = self.file_type();
        if file_type.is_dir() {
            r#type |= FileMode::DIR.bits();
        } else if file_type.is_symlink() {
            r#type |= FileMode::LNK.bits();
        } else if file_type.is_socket() {
            r#type |= FileMode::SOCK.bits();
        } else if file_type.is_block_device() {
            r#type |= FileMode::BLK.bits();
        } else if file_type.is_char_device() {
            r#type |= FileMode::CHR.bits();
        } else if file_type.is_fifo() {
            r#type |= FileMode::FIFO.bits();
        } else {
            r#type |= FileMode::REG.bits();
        }
        FileResponse {
            name: filename,
            path,
            mode: self.mode() | r#type,
            size: self.size(),
            uid: self.uid(),
            gid: self.gid(),
        }
    }
}
