use std::collections::HashMap;

use proc_macro_error::*;
use syn::*;

#[allow(dead_code)]
pub fn to_camel(ident: &Ident) -> Ident {
    Ident::new(
        ident
            .to_string()
            .chars()
            .fold((String::new(), '_'), |(mut ret, prev), this| {
                if this != '_' {
                    if prev == '_' {
                        ret.extend(this.to_uppercase());
                    } else if prev.is_uppercase() {
                        ret.extend(this.to_lowercase());
                    } else {
                        ret.push(this);
                    }
                }
                (ret, this)
            })
            .0
            .as_str(),
        ident.span(),
    )
}

#[allow(dead_code)]
pub fn to_lower_snake(ident: &Ident) -> Ident {
    Ident::new(
        ident
            .to_string()
            .chars()
            .fold((String::new(), '_'), |(mut ret, prev), this| {
                if this.is_uppercase() && prev != '_' {
                    ret.push('_');
                }
                ret.extend(this.to_lowercase());
                (ret, this)
            })
            .0
            .as_str(),
        ident.span(),
    )
}

pub type FlagsMap = HashMap<String, Ident>;
pub type PropsMap = HashMap<String, (Ident, Lit)>;

#[allow(dead_code)]
pub fn parse_meta(attr: &Attribute) -> Option<(FlagsMap, PropsMap)> {
    let (mut flags, mut props) = Default::default();
    let meta = attr.parse_meta().ok()?;
    match meta {
        Meta::Path(p) => {
            debug_assert!(p.is_ident("rpc"));
            return Some((flags, props));
        }
        Meta::List(l) => {
            debug_assert!(l.path.is_ident("rpc"));
            for meta in l.nested.into_iter() {
                match meta {
                    NestedMeta::Meta(Meta::NameValue(v)) => {
                        let prop_ident = v.path.get_ident()?.clone();
                        let span = prop_ident.span();
                        if let Some((orig_ident, _)) =
                            props.insert(prop_ident.to_string(), (prop_ident, v.lit))
                        {
                            emit_warning!(
                                span, "duplicate property";
                                note = orig_ident.span() => "previous here";
                            );
                        }
                    }
                    NestedMeta::Meta(Meta::Path(p)) => {
                        let flag_ident = p.get_ident()?.clone();
                        let span = flag_ident.span();
                        if let Some(orig_ident) = flags.insert(flag_ident.to_string(), flag_ident) {
                            emit_warning!(
                                span, "duplicate flags";
                                note = orig_ident.span() => "previous here";
                            );
                        }
                    }
                    _ => return None,
                };
            }
        }
        _ => return None,
    }
    Some((flags, props))
}
