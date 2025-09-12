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
use proc_macro2::{Group, Ident, TokenStream};
use syn::LitStr;

#[derive(Debug)]
#[expect(
    clippy::arbitrary_source_item_ordering,
    reason = "In the order of occurrence"
)]
pub enum ParseExpectation {
    FnKeyword,
    FunctionIdentifier {
        fn_keyword: Ident,
    },
    ArgumentsTuple {
        fn_keyword: Ident,
        fn_ident: Ident,
    },
    FunctionOrResult {
        fn_keyword: Ident,
        fn_ident: Ident,
        fn_args: Group,
    },
    Arrow {
        fn_keyword: Ident,
        fn_ident: Ident,
        fn_args: Group,
    },
    Result {
        fn_keyword: Ident,
        fn_ident: Ident,
        fn_args: Group,
    },
}

#[derive(Clone)]
pub enum FunctionArgument {
    Special {
        variable_name: Ident,
        variable_type: TokenStream,
    },
    Header {
        variable_name: Ident,
        variable_type: TokenStream,
        variable_type_wopt: TokenStream,
        required: bool,
    },
    Path {
        variable_name: Ident,
        variable_type: TokenStream,
    },
    Payload {
        variable_name: Ident,
        variable_type: TokenStream,
    },
    Query {
        variable_name: Ident,
        variable_type_wopt: TokenStream,
        required: bool,
        desc: Option<LitStr>,
    },
}
