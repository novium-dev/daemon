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
use crate::service::process::HttpMethod;
use core::str::FromStr as _;
use proc_macro2::{Delimiter, Group, Ident, Punct, Span};
use std::collections::HashMap;
use syn::parse::{Parse, ParseStream};
use syn::{LitStr, Token};

pub struct RouteArgs {
    pub absolute: bool,
    pub method: HttpMethod,
    pub path: String,
    pub query: HashMap<String, Option<LitStr>>,
    pub no_auth: bool,
    pub raw: bool,
    pub routes: Vec<Ident>,
}

impl Parse for RouteArgs {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let absolute = input.peek(Token![!]);
        if absolute {
            let _: Punct = input.parse()?;
        }
        let path: LitStr = input.parse()?;
        let mut method = HttpMethod::Get;
        let mut query = HashMap::new();
        let mut routes = vec![Ident::new("default", Span::call_site())];
        let mut raw = false;
        let mut no_auth = false;

        if input.peek(Token![,]) {
            let _: Punct = input.parse()?;
            for (method_opt, query_opt, routes_opt, raw_opt, no_auth_opt) in
                input.parse_terminated(parse_route_args_option, Token![,])?
            {
                if let Some(opt) = method_opt {
                    method = opt;
                }
                if let Some(opt) = routes_opt {
                    routes = opt;
                }
                raw |= raw_opt;
                no_auth |= no_auth_opt;
                query.extend(query_opt);
            }
        }

        routes.push(Ident::new(
            if no_auth { "with_auth" } else { "without_auth" },
            Span::call_site(),
        ));

        Ok(Self {
            absolute,
            path: path.value(),
            method,
            query,
            routes,
            raw,
            no_auth,
        })
    }
}

struct QueryArgs(Vec<(Ident, Option<LitStr>)>);

impl Parse for QueryArgs {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        input
            .parse_terminated(parse_query_arg, Token![,])
            .map(|result| Self(result.into_iter().collect()))
    }
}

struct RoutesArgs(Vec<Ident>);

impl Parse for RoutesArgs {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        input
            .parse_terminated(parse_routes_arg, Token![,])
            .map(|result| Self(result.into_iter().collect()))
    }
}

#[expect(clippy::type_complexity)]
fn parse_route_args_option(
    stream: ParseStream,
) -> Result<
    (
        Option<HttpMethod>,
        HashMap<String, Option<LitStr>>,
        Option<Vec<Ident>>,
        bool,
        bool,
    ),
    syn::Error,
> {
    let opt: Ident = stream.parse()?;
    if opt.to_string().eq("raw") {
        return Ok((None, HashMap::new(), None, true, false));
    }
    if opt.to_string().eq("no_auth") {
        return Ok((None, HashMap::new(), None, false, true));
    }
    if stream.peek(Token![=]) {
        let _: Punct = stream.parse()?;
        match opt.to_string().as_str() {
            "method" => {
                let method_ident: Ident = stream.parse()?;
                match HttpMethod::from_str(&method_ident.to_string()) {
                    Ok(val) => Ok((Some(val), HashMap::new(), None, false, false)),
                    Err(()) => Err(stream.error("Unknown method")),
                }
            }
            "query" => {
                let query_group: Group = stream.parse()?;
                if query_group.delimiter() != Delimiter::Bracket {
                    return Err(stream.error("Expected brackets"));
                }
                Ok((
                    None,
                    syn::parse2::<QueryArgs>(query_group.stream())?
                        .0
                        .into_iter()
                        .map(|(arg, desc)| (arg.to_string(), desc))
                        .collect(),
                    None,
                    false,
                    false,
                ))
            }
            "routes" => {
                let routes_group: Group = stream.parse()?;
                if routes_group.delimiter() != Delimiter::Bracket {
                    return Err(stream.error("Expected brackets"));
                }
                Ok((
                    None,
                    HashMap::new(),
                    Some(
                        syn::parse2::<RoutesArgs>(routes_group.stream())?
                            .0
                            .into_iter()
                            .collect(),
                    ),
                    false,
                    false,
                ))
            }
            _ => Err(stream.error("Unknown option")),
        }
    } else {
        Err(stream.error("Expected ="))
    }
}

fn parse_query_arg(stream: ParseStream) -> Result<(Ident, Option<LitStr>), syn::Error> {
    let arg: Ident = stream.parse()?;
    #[expect(
        clippy::if_then_some_else_none,
        reason = "False positive. It does have a question-mark"
    )]
    let desc = if stream.peek(syn::LitStr) {
        Some(stream.parse()?)
    } else {
        None
    };
    Ok((arg, desc))
}

fn parse_routes_arg(stream: ParseStream) -> Result<Ident, syn::Error> {
    stream.parse()
}
