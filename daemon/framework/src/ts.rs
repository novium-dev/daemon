use crate::{FileResponse, MaybeUnit};
use convert_case::{Case, Casing};
use std::any::TypeId;
use std::collections::HashSet;
use std::marker::PhantomData;
use ts_rs::{TS, TypeVisitor};

pub struct TSResponse<T>(PhantomData<T>);

#[derive(Default)]
pub struct TSTypeVisitor {
    pub out: String,
    pub exported: HashSet<TypeId>,
    pub allowed: HashSet<TypeId>,
    pub json_value: bool,
}

impl TypeVisitor for TSTypeVisitor {
    fn visit<T: TS + 'static + ?Sized>(&mut self) {
        if !self.allowed.contains(&TypeId::of::<T>()) {
            return;
        }
        if self.exported.insert(TypeId::of::<T>()) {
            if T::name().eq("JsonValue") {
                if self.json_value {
                    return;
                }
                self.json_value = true;
            }

            self.allowed
                .extend(T::dependencies().into_iter().map(|dep| dep.type_id));
            T::visit_dependencies(self);

            let docs = &T::DOCS;
            if let Some(docs) = docs {
                self.out.push_str(docs);
            }

            // Type Definition
            self.out.push_str("export ");
            self.out.push_str(&T::decl());
            self.out.push('\n');
        }
    }
}

#[derive(Default)]
pub struct TSClassFunctionVisitor {
    pub out: String,
    pub r#async: bool,
}

impl TSClassFunctionVisitor {
    pub fn visit_doc(&mut self, doc: impl Into<String>) {
        let doc = doc.into();
        self.out.push_str("  /** ");
        self.out.push_str(&doc);
        self.out.push_str(" */\n");
    }

    pub fn visit_start(&mut self, r#async: bool, name: impl Into<String>) {
        self.out.push_str("  ");
        if r#async {
            self.out.push_str("async ");
        }
        self.r#async = r#async;
        self.out.push_str(&name.into());
        self.out.push('(');
    }

    pub fn visit_argument<T: TS>(&mut self, name: impl Into<String>) {
        self.out.push_str(&name.into());
        self.out.push_str(": ");
        self.out.push_str(&T::name());
        self.out.push_str(", ");
    }

    pub fn visit_end_result_custom(&mut self, body: impl Into<String>, result: &str) {
        let body = body.into();
        if let Some(out) = self.out.strip_suffix(", ") {
            self.out = String::from(out);
        }
        self.out.push_str("): ");
        if self.r#async {
            self.out.push_str("Promise<");
        }
        self.out.push_str(result);
        if self.r#async {
            self.out.push('>');
        }
        self.out.push_str(" {\n");
        for line in body.lines() {
            if line.is_empty() {
                self.out.push('\n');
            } else {
                self.out.push_str("    ");
                self.out.push_str(line);
                self.out.push('\n');
            }
        }
        self.out.push_str("  ");
        self.out.push_str("}\n\n");
    }

    pub fn visit_end_result<T: TS>(&mut self, body: impl Into<String>) {
        self.visit_end_result_custom(body, &T::name());
    }

    pub fn visit_end(&mut self, body: impl Into<String>) {
        let body = body.into();
        if let Some(out) = self.out.strip_suffix(", ") {
            self.out = String::from(out);
        }
        self.out.push_str(") {\n");
        for line in body.lines() {
            if line.is_empty() {
                self.out.push('\n');
            } else {
                self.out.push_str("    ");
                self.out.push_str(line);
                self.out.push('\n');
            }
        }
        self.out.push_str("  ");
        self.out.push_str("}\n\n");
    }
}

pub trait FrameworkTSResponse {
    fn visit_ts_response() -> String;
}

impl<V: MaybeUnit + ts_rs::TS, E> FrameworkTSResponse for Result<V, E> {
    fn visit_ts_response() -> String {
        if V::unit().is_some() {
            "return { Ok: null };".to_string()
        } else {
            format!("return {{ Ok: await response.json() as {} }};", V::name())
        }
    }
}

impl<E> FrameworkTSResponse for Result<FileResponse, E> {
    fn visit_ts_response() -> String {
        "return { Ok: await response.blob() };".to_string()
    }
}

impl<E: TS> TS for TSResponse<Result<FileResponse, E>> {
    type WithoutGenerics = Self;

    fn decl() -> String {
        panic!("{} cannot be declared", Self::name())
    }

    fn decl_concrete() -> String {
        Self::decl()
    }

    fn name() -> String {
        format!("{{ Ok : Blob }} | {{ Err : {} }}", E::name())
    }

    fn inline() -> String {
        format!("{{ Ok : Blob }} | {{ Err : {} }}", E::inline())
    }

    fn inline_flattened() -> String {
        panic!("{} cannot be flattened", Self::name())
    }

    fn visit_dependencies(v: &mut impl TypeVisitor)
    where
        Self: 'static,
    {
        E::visit_dependencies(v);
    }

    fn visit_generics(v: &mut impl TypeVisitor)
    where
        Self: 'static,
    {
        E::visit_generics(v);
        v.visit::<E>();
    }
}

pub fn to_camel(str: &str) -> String {
    str.to_case(Case::Camel)
}
