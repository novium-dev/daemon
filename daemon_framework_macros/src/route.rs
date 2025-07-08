use crate::service::process::HttpMethod;
use core::str::FromStr as _;
use proc_macro2::{Delimiter, Group, Ident, Punct};
use std::collections::HashMap;
use syn::parse::{Parse, ParseStream};
use syn::{LitStr, Token};

pub struct RouteArgs {
    pub absolute: bool,
    pub method: HttpMethod,
    pub path: String,
    pub query: HashMap<String, Option<LitStr>>,
    pub raw: bool,
    pub user: bool,
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
        let mut user = false;
        let mut raw = false;

        if input.peek(Token![,]) {
            let _: Punct = input.parse()?;
            for (method_opt, query_opt, user_opt, raw_opt) in
                input.parse_terminated(parse_route_args_option, Token![,])?
            {
                if let Some(opt) = method_opt {
                    method = opt;
                }
                if let Some(opt) = user_opt {
                    user = opt;
                }
                if let Some(opt) = raw_opt {
                    raw = opt;
                }
                query.extend(query_opt);
            }
        }

        Ok(Self {
            absolute,
            path: path.value(),
            method,
            query,
            user,
            raw,
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

#[expect(clippy::type_complexity)]
fn parse_route_args_option(
    stream: ParseStream,
) -> Result<
    (
        Option<HttpMethod>,
        HashMap<String, Option<LitStr>>,
        Option<bool>,
        Option<bool>,
    ),
    syn::Error,
> {
    let opt: Ident = stream.parse()?;
    if opt.to_string().eq("user") {
        return Ok((None, HashMap::new(), Some(true), None));
    }
    if opt.to_string().eq("raw") {
        return Ok((None, HashMap::new(), None, Some(true)));
    }
    if stream.peek(Token![=]) {
        let _: Punct = stream.parse()?;
        match opt.to_string().as_str() {
            "method" => {
                let method_ident: Ident = stream.parse()?;
                match HttpMethod::from_str(&method_ident.to_string()) {
                    Ok(val) => Ok((Some(val), HashMap::new(), None, None)),
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
                    None,
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
