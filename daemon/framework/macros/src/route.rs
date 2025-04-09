use crate::service::process::HttpMethod;
use crate::utils::TokenArray;
use proc_macro2::{Delimiter, Group, Ident, Punct};
use std::collections::HashSet;
use std::str::FromStr;
use syn::parse::{Parse, ParseStream};
use syn::{LitStr, Token};

pub(crate) struct RouteArgs {
    pub(crate) absolute: bool,
    pub(crate) path: String,
    pub(crate) method: HttpMethod,
    pub(crate) query: HashSet<String>,
    pub(crate) user: bool,
}

#[allow(clippy::type_complexity)]
fn parse_route_args_option(
    stream: ParseStream,
) -> Result<(Option<HttpMethod>, HashSet<String>, Option<bool>), syn::Error> {
    let opt: Ident = stream.parse()?;
    if opt.to_string().eq("user") {
        return Ok((None, HashSet::new(), Some(true)));
    }
    if stream.peek(Token![=]) {
        let _: Punct = stream.parse()?;
        match opt.to_string().as_str() {
            "method" => {
                let method_ident: Ident = stream.parse()?;
                match HttpMethod::from_str(&method_ident.to_string()) {
                    Ok(val) => Ok((Some(val), HashSet::new(), None)),
                    Err(_) => Err(stream.error("Unknown method")),
                }
            }
            "query" => {
                let query_group: Group = stream.parse()?;
                if query_group.delimiter() != Delimiter::Bracket {
                    return Err(stream.error("Expected brackets"));
                }
                Ok((
                    None,
                    syn::parse2::<TokenArray<Ident>>(query_group.stream())?
                        .0
                        .into_iter()
                        .map(|i| i.to_string())
                        .collect(),
                    None,
                ))
            }
            _ => Err(stream.error("Unknown option")),
        }
    } else {
        Err(stream.error("Expected ="))
    }
}

impl Parse for RouteArgs {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let absolute = input.peek(Token![!]);
        if absolute {
            let _: Punct = input.parse()?;
        }
        let path: LitStr = input.parse()?;
        let mut method = HttpMethod::GET;
        let mut query = HashSet::new();
        let mut user = false;

        if input.peek(Token![,]) {
            let _: Punct = input.parse()?;
            for (method_opt, query_opt, auth_opt) in
                input.parse_terminated(parse_route_args_option, Token![,])?
            {
                if let Some(opt) = method_opt {
                    method = opt;
                }
                if let Some(opt) = auth_opt {
                    user = opt;
                }
                query.extend(query_opt);
            }
        }

        Ok(RouteArgs {
            absolute,
            path: path.value(),
            method,
            query,
            user,
        })
    }
}
