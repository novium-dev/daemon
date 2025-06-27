use proc_macro2::{Ident, Punct};
use syn::parse::{Parse, ParseStream};
use syn::{LitStr, Token};

pub struct ServiceArgs {
    pub path: String,
    pub tag: Option<LitStr>,
}

impl Parse for ServiceArgs {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let mut path = None;
        let mut tag = None;
        for (path_opt, tag_opt) in input.parse_terminated(parse_service_args_option, Token![,])? {
            if let Some(opt) = path_opt {
                path = Some(opt);
            }
            if let Some(opt) = tag_opt {
                tag = Some(opt);
            }
        }

        Ok(Self {
            path: path.unwrap_or_default(),
            tag,
        })
    }
}

fn parse_service_args_option(
    stream: ParseStream,
) -> Result<(Option<String>, Option<LitStr>), syn::Error> {
    let opt: Ident = stream.parse()?;
    if stream.peek(Token![=]) {
        let _: Punct = stream.parse()?;
        match opt.to_string().as_str() {
            "path" => {
                let path_literal: LitStr = stream.parse()?;
                Ok((Some(path_literal.value()), None))
            }
            "tag" => {
                let tag_literal: LitStr = stream.parse()?;

                Ok((None, Some(tag_literal)))
            }
            _ => Err(stream.error("Unknown option")),
        }
    } else {
        Err(stream.error("Expected ="))
    }
}
