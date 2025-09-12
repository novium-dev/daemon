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
use proc_macro2::{Ident, Punct};
use syn::parse::{Parse, ParseStream};
use syn::{LitStr, Token};

pub struct ServiceArgs {
    pub path: String,
    pub tag: Option<LitStr>,
    pub no_sdk: bool,
}

impl Parse for ServiceArgs {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let mut path = None;
        let mut tag = None;
        let mut no_sdk = false;
        for (path_opt, tag_opt, no_sdk_opt) in
            input.parse_terminated(parse_service_args_option, Token![,])?
        {
            if let Some(opt) = path_opt {
                path = Some(opt);
            }
            if let Some(opt) = tag_opt {
                tag = Some(opt);
            }
            if no_sdk_opt {
                no_sdk = true;
            }
        }

        Ok(Self {
            path: path.unwrap_or_default(),
            tag,
            no_sdk,
        })
    }
}

fn parse_service_args_option(
    stream: ParseStream,
) -> Result<(Option<String>, Option<LitStr>, bool), syn::Error> {
    let opt: Ident = stream.parse()?;

    #[expect(clippy::single_match, reason = "maybe more options later")]
    match opt.to_string().as_str() {
        "no_sdk" => return Ok((None, None, true)),
        _ => {}
    }
    if stream.peek(Token![=]) {
        let _: Punct = stream.parse()?;
        match opt.to_string().as_str() {
            "path" => {
                let path_literal: LitStr = stream.parse()?;
                Ok((Some(path_literal.value()), None, false))
            }
            "tag" => {
                let tag_literal: LitStr = stream.parse()?;

                Ok((None, Some(tag_literal), false))
            }
            _ => Err(stream.error("Unknown option")),
        }
    } else {
        Err(stream.error("Expected ="))
    }
}
