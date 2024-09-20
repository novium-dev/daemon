use diesel::result::Error;
use diesel::{
    define_sql_function, Associations, BelongingToDsl, Identifiable, Insertable, MysqlConnection,
    Queryable, RunQueryDsl, Selectable,
};
use std::collections::HashMap;

#[derive(Clone, Default)]
pub(crate) struct FetchedServer {
    pub(crate) id: u32,

    pub(crate) build_dockerfile: String,
    pub(crate) build_args: HashMap<String, String>,
    pub(crate) build_identifier: String,

    pub(crate) cpu: Option<f32>,
    pub(crate) ram: Option<u32>,
    pub(crate) swap: Option<u32>,
    pub(crate) disk: Option<u64>,

    pub(crate) cap_add: Vec<String>,
    pub(crate) cap_drop: Vec<String>,

    pub(crate) ports: Vec<Port>,
}

impl Server {
    pub(crate) fn fetched(self, conn: &mut MysqlConnection) -> Result<FetchedServer, Error> {
        let build_args = HashMap::from_iter(
            ServerBuildArg::belonging_to(&self)
                .load::<ServerBuildArg>(conn)?
                .into_iter()
                .map(|arg| (arg.key, arg.value)),
        );

        let ports = Port::belonging_to(&self).load::<Port>(conn)?;

        let cap_add = CapabilityAdd::belonging_to(&self)
            .load::<CapabilityAdd>(conn)?
            .into_iter()
            .map(|cap_add| cap_add.capability)
            .collect();

        let cap_drop = CapabilityDrop::belonging_to(&self)
            .load::<CapabilityDrop>(conn)?
            .into_iter()
            .map(|cap_drop| cap_drop.capability)
            .collect();

        Ok(FetchedServer {
            id: self.id,
            build_dockerfile: self.build_dockerfile,
            build_args,
            build_identifier: self.build_identifier,

            cpu: self.cpu,
            ram: self.ram,
            swap: self.swap,
            disk: self.disk,

            ports,

            cap_add,
            cap_drop,
        })
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
    pub(crate) disk: Option<u64>, // in MiB, only working when using btrfs
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

#[derive(Identifiable, Insertable, Queryable, Selectable, Associations, Clone)]
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
