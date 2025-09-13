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
#![allow(clippy::expect_used, reason = "This is a macro. Panics are fine.")]
#![allow(clippy::panic, reason = "This is a macro. Panics are fine.")]
#![forbid(
    clippy::inline_always,
    reason = "This is a macro. Compile time is more important here than minor speed improvements."
)]

mod error;
mod route;
mod service;
mod utils;

use crate::error::define::define_error0;
use crate::error::error0;
use crate::service::args::ServiceArgs;
use crate::service::service0;
use manyhow::manyhow;
use proc_macro::TokenStream as StdTokenStream;
use proc_macro2::TokenStream;
use quote::quote;
use syn::parse_macro_input;

#[proc_macro_attribute]
pub fn service(attr: StdTokenStream, input: StdTokenStream) -> StdTokenStream {
    let args = parse_macro_input!(attr as ServiceArgs);
    StdTokenStream::from(service0(&crate_prefix(), &args, TokenStream::from(input)))
}

#[manyhow]
#[proc_macro]
pub fn error(attr: TokenStream) -> syn::Result<TokenStream> {
    error0(&crate_prefix(), attr)
}

#[proc_macro]
pub fn define_error(attr: StdTokenStream) -> StdTokenStream {
    StdTokenStream::from(define_error0(&crate_prefix(), TokenStream::from(attr)))
}

fn crate_prefix() -> TokenStream {
    quote! {
        ::daemon_framework::
    }
}
