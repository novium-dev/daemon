use base64::prelude::BASE64_STANDARD;
use base64::Engine;
use daemon_common::payload::server::ServerFileWrite;
use diesel::result::Error;
use diesel::{
    define_sql_function, Associations, BelongingToDsl, Identifiable, Insertable, Queryable,
    Selectable,
};
use diesel_async::{AsyncMysqlConnection, RunQueryDsl};
use std::collections::{HashMap, HashSet};
use std::io;
use std::io::ErrorKind;

#[cfg(feature = "database")]
#[derive(Clone, Default, Eq, PartialEq)]
pub struct FetchedDatabase {
    pub name_suffix: String,
    pub passwordless: HashMap<u32, i32>,
    pub users: HashMap<String, DatabaseUser>,
}

#[cfg(feature = "database")]
impl Database {
    pub(crate) async fn fetched(
        self,
        conn: &mut AsyncMysqlConnection,
    ) -> Result<FetchedDatabase, Error> {
        let passwordless = DatabasePasswordless::belonging_to(&self)
            .load::<DatabasePasswordless>(conn)
            .await?
            .into_iter()
            .map(|passwordless| (passwordless.server_id, passwordless.permissions))
            .collect();
        let users = DatabaseUser::belonging_to(&self)
            .load::<DatabaseUser>(conn)
            .await?
            .into_iter()
            .map(|user| (user.name_suffix.clone(), user))
            .collect();

        Ok(FetchedDatabase {
            name_suffix: self.name_suffix,
            passwordless,
            users,
        })
    }
}

#[derive(Debug, Clone, Default)]
pub(crate) struct FetchedServer {
    pub(crate) id: u32,

    pub(crate) build_dockerfile: String,
    pub(crate) build_args: HashMap<String, String>,
    pub(crate) build_identifier: String,
    pub(crate) backup_exclude: HashSet<String>,
    pub(crate) files_write: HashMap<String, ServerFileWrite>,

    pub(crate) cpu: Option<f32>,
    pub(crate) ram: Option<u32>,
    pub(crate) swap: Option<u32>,
    pub(crate) disk: Option<u64>,

    pub(crate) cap_add: Vec<String>,
    pub(crate) cap_drop: Vec<String>,

    pub(crate) ports: Vec<Port>,
    pub(crate) storage: String,
}

impl Server {
    pub(crate) async fn fetched(
        self,
        conn: &mut AsyncMysqlConnection,
    ) -> Result<FetchedServer, Error> {
        let build_args = HashMap::from_iter(
            ServerBuildArg::belonging_to(&self)
                .load::<ServerBuildArg>(conn)
                .await?
                .into_iter()
                .map(|arg| (arg.key, arg.value)),
        );
        let backup_exclude = HashSet::from_iter(
            ServerBackupExclude::belonging_to(&self)
                .load::<ServerBackupExclude>(conn)
                .await?
                .into_iter()
                .map(|arg| arg.exclude),
        );

        let ports = Port::belonging_to(&self).load::<Port>(conn).await?;

        let cap_add = CapabilityAdd::belonging_to(&self)
            .load::<CapabilityAdd>(conn)
            .await?
            .into_iter()
            .map(|cap_add| cap_add.capability)
            .collect();

        let cap_drop = CapabilityDrop::belonging_to(&self)
            .load::<CapabilityDrop>(conn)
            .await?
            .into_iter()
            .map(|cap_drop| cap_drop.capability)
            .collect();

        let files_write_entries: Vec<ServerFileWriteEntry> =
            ServerFileWriteEntry::belonging_to(&self)
                .load::<ServerFileWriteEntry>(conn)
                .await?;
        let mut files_write: HashMap<String, ServerFileWrite> = HashMap::new();
        for entry in files_write_entries {
            match entry.filetype.as_str() {
                "properties" => {
                    if !files_write.contains_key(&entry.file) {
                        files_write.insert(
                            entry.file.clone(),
                            ServerFileWrite::PROPERTIES {
                                content: HashMap::new(),
                            },
                        );
                    }
                    if let ServerFileWrite::PROPERTIES { content } =
                        files_write.get_mut(&entry.file).unwrap()
                    {
                        content.insert(
                            entry
                                .key
                                .ok_or(diesel::result::Error::DeserializationError(Box::new(
                                    io::Error::new(ErrorKind::Other, "key is null!"),
                                )))?,
                            entry.value,
                        );
                    }
                }
                "write" => {
                    files_write.insert(
                        entry.file.clone(),
                        ServerFileWrite::WRITE {
                            data: BASE64_STANDARD
                                .decode(entry.value)
                                .map_err(Box::new)
                                .map_err(|err| diesel::result::Error::DeserializationError(err))?,
                        },
                    );
                }
                _ => {}
            }
        }

        Ok(FetchedServer {
            id: self.id,
            build_dockerfile: self.build_dockerfile,
            build_args,
            build_identifier: self.build_identifier,
            backup_exclude,
            files_write,

            cpu: self.cpu,
            ram: self.ram,
            swap: self.swap,
            disk: self.disk,

            ports,

            cap_add,
            cap_drop,

            storage: self.storage,
        })
    }
}

#[cfg(feature = "database")]
#[derive(Clone, Eq, PartialEq, Insertable, Queryable, Selectable, Identifiable)]
#[diesel(table_name = databases)]
#[diesel(primary_key(id))]
pub(crate) struct Database {
    pub(crate) id: u32,
    pub name_suffix: String,
}

#[cfg(feature = "database")]
diesel::table! {
    databases (id) {
        id -> Unsigned<Integer>,
        name_suffix -> VarChar,
    }
}

#[cfg(feature = "database")]
#[derive(Clone, Eq, PartialEq, Identifiable, Insertable, Queryable, Selectable, Associations)]
#[diesel(table_name = database_passwordless)]
#[diesel(belongs_to(Database, foreign_key = database_id))]
#[diesel(primary_key(id))]
pub(crate) struct DatabasePasswordless {
    pub(crate) id: u32,
    pub(crate) database_id: u32,
    pub(crate) server_id: u32,
    pub(crate) permissions: i32,
}

#[cfg(feature = "database")]
diesel::table! {
    database_passwordless (id) {
        id -> Unsigned<Integer>,
        database_id -> Unsigned<Integer>,
        server_id -> Unsigned<Integer>,
        permissions -> Integer
    }
}

#[cfg(feature = "database")]
#[derive(Clone, Eq, PartialEq, Identifiable, Insertable, Queryable, Selectable, Associations)]
#[diesel(table_name = database_user)]
#[diesel(belongs_to(Database, foreign_key = database_id))]
#[diesel(primary_key(id))]
pub(crate) struct DatabaseUser {
    pub(crate) id: u32,
    pub(crate) database_id: u32,
    pub(crate) name_suffix: String,
    pub(crate) password_hash: String,
    pub(crate) permissions: i32,
}

#[cfg(feature = "database")]
diesel::table! {
    database_user (id) {
        id -> Unsigned<Integer>,
        database_id -> Unsigned<Integer>,
        name_suffix -> VarChar,
        password_hash -> VarChar,
        permissions -> Integer
    }
}

#[derive(Insertable, Queryable, Selectable, Identifiable)]
#[diesel(table_name = servers)]
#[diesel(primary_key(id))]
pub(crate) struct Server {
    pub(crate) id: u32,

    pub(crate) build_dockerfile: String,
    pub(crate) build_identifier: String,

    pub(crate) cpu: Option<f32>,  // in Threads
    pub(crate) ram: Option<u32>,  // in MiB
    pub(crate) swap: Option<u32>, // in MiB
    pub(crate) disk: Option<u64>, // in MiB

    pub(crate) storage: String,
}

diesel::table! {
    servers (id) {
        id -> Unsigned<Integer>,

        build_dockerfile -> VarChar,
        build_identifier -> VarChar,

        cpu -> Nullable<Float4>,
        ram -> Nullable<Unsigned<Integer>>,
        swap -> Nullable<Unsigned<Integer>>,
        disk -> Nullable<Unsigned<BigInt>>,

        storage -> VarChar,
    }
}

#[derive(Identifiable, Insertable, Queryable, Selectable, Associations)]
#[diesel(table_name = server_capadd)]
#[diesel(belongs_to(Server, foreign_key = server_id))]
#[diesel(primary_key(id))]
pub(crate) struct CapabilityAdd {
    pub(crate) id: u32,
    pub(crate) server_id: u32,
    pub(crate) capability: String,
}

diesel::table! {
    server_capadd (id) {
        id -> Unsigned<Integer>,
        server_id -> Unsigned<Integer>,
        capability -> VarChar,
    }
}

#[derive(Identifiable, Insertable, Queryable, Selectable, Associations)]
#[diesel(table_name = server_capdrop)]
#[diesel(belongs_to(Server, foreign_key = server_id))]
#[diesel(primary_key(id))]
pub(crate) struct CapabilityDrop {
    pub(crate) id: u32,
    pub(crate) server_id: u32,
    pub(crate) capability: String,
}

diesel::table! {
    server_capdrop (id) {
        id -> Unsigned<Integer>,
        server_id -> Unsigned<Integer>,
        capability -> VarChar,
    }
}

#[derive(Identifiable, Insertable, Queryable, Selectable, Associations)]
#[diesel(table_name = server_backup_exclude)]
#[diesel(belongs_to(Server, foreign_key = server_id))]
#[diesel(primary_key(id))]
pub(crate) struct ServerBackupExclude {
    pub(crate) id: u32,
    pub(crate) server_id: u32,
    pub(crate) exclude: String,
}

diesel::table! {
    server_backup_exclude (id) {
        id -> Unsigned<Integer>,
        server_id -> Unsigned<Integer>,
        exclude -> VarChar,
    }
}

#[derive(Identifiable, Insertable, Queryable, Selectable, Associations)]
#[diesel(table_name = server_file_writes)]
#[diesel(belongs_to(Server, foreign_key = server_id))]
#[diesel(primary_key(server_id, file))]
pub(crate) struct ServerFileWriteEntry {
    pub(crate) server_id: u32,
    pub(crate) file: String,
    pub(crate) filetype: String,
    pub(crate) key: Option<String>,
    pub(crate) value: String,
}

diesel::table! {
    server_file_writes (server_id, file) {
        server_id -> Unsigned<Integer>,
        file -> VarChar,
        filetype -> VarChar,
        key -> Nullable<VarChar>,
        value -> VarChar
    }
}

#[derive(Identifiable, Insertable, Queryable, Selectable, Associations)]
#[diesel(table_name = server_buildargs)]
#[diesel(belongs_to(Server, foreign_key = server_id))]
#[diesel(primary_key(id))]
pub(crate) struct ServerBuildArg {
    pub(crate) id: u32,
    pub(crate) server_id: u32,
    pub(crate) key: String,
    pub(crate) value: String,
}

diesel::table! {
    server_buildargs (id) {
        id -> Unsigned<Integer>,
        server_id -> Unsigned<Integer>,
        key -> VarChar,
        value -> VarChar,
    }
}

#[derive(Debug, Identifiable, Insertable, Queryable, Selectable, Associations, Clone)]
#[diesel(table_name = server_ports)]
#[diesel(belongs_to(Server, foreign_key = server_id))]
#[diesel(primary_key(id))]
pub(crate) struct Port {
    pub(crate) id: u32,
    pub(crate) server_id: u32,
    pub(crate) internal_port: u16,
    pub(crate) external_port: u16,
    pub(crate) external_ip: String,
    pub(crate) protocol: String,
}

diesel::table! {
    server_ports (id) {
        id -> Unsigned<Integer>,
        server_id -> Unsigned<Integer>,
        internal_port -> Unsigned<SmallInt>,
        external_port -> Unsigned<SmallInt>,
        external_ip -> VarChar,
        protocol -> VarChar,
    }
}

define_sql_function! {
    fn last_insert_id() -> Unsigned<BigInt>
}
