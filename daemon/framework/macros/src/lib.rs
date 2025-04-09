mod error;
mod route;
mod service;
mod utils;
mod variable;

use crate::service::args::ServiceArgs;
use proc_macro2::TokenStream;
use quote::quote;
use syn::parse_macro_input;

#[proc_macro_attribute]
pub fn service(
    attr: proc_macro::TokenStream,
    input: proc_macro::TokenStream,
) -> proc_macro::TokenStream {
    let args = parse_macro_input!(attr as ServiceArgs);
    proc_macro::TokenStream::from(crate::service::service0(
        crate_prefix(),
        args,
        proc_macro2::TokenStream::from(input),
    ))
}

#[proc_macro]
pub fn error(attr: proc_macro::TokenStream) -> proc_macro::TokenStream {
    proc_macro::TokenStream::from(crate::error::error0(
        crate_prefix(),
        proc_macro2::TokenStream::from(attr),
    ))
}

#[proc_macro]
pub fn define_error(attr: proc_macro::TokenStream) -> proc_macro::TokenStream {
    proc_macro::TokenStream::from(crate::error::define::define_error0(
        crate_prefix(),
        proc_macro2::TokenStream::from(attr),
    ))
}

#[proc_macro]
pub fn define_variable(attr: proc_macro::TokenStream) -> proc_macro::TokenStream {
    proc_macro::TokenStream::from(crate::variable::define_variable0(
        crate_prefix(),
        proc_macro2::TokenStream::from(attr),
    ))
}

fn crate_prefix() -> TokenStream {
    quote! {
        daemon_framework::
    }
}
