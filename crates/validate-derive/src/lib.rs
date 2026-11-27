//! `#[derive(Validate)]`: rules as attributes on a struct's fields.
//! Use it through the `validate` crate's `derive` feature; see its docs.

use proc_macro::TokenStream;
use proc_macro2::{Span, TokenStream as Tokens};
use quote::quote;
use syn::{
    Data, DeriveInput, Error, Expr, Fields, Ident, LitStr, Path, Token, Type, parenthesized, parse_macro_input,
    punctuated::Punctuated,
};

#[proc_macro_derive(Validate, attributes(validate))]
pub fn derive(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    expand(&input).unwrap_or_else(Error::into_compile_error).into()
}

/// One rule from `#[validate(...)]`, in the order written.
enum RuleAttr {
    Required,
    Rule(Tokens),
    With(Path),
}

struct FieldInfo<'a> {
    ident: &'a Ident,
    ty: &'a Type,
    key: String,
    rules: Vec<RuleAttr>,
}

fn expand(input: &DeriveInput) -> syn::Result<Tokens> {
    let Data::Struct(data) = &input.data else {
        return Err(Error::new_spanned(input, "#[derive(Validate)] works on structs with named fields"));
    };
    let Fields::Named(named) = &data.fields else {
        return Err(Error::new_spanned(input, "#[derive(Validate)] works on structs with named fields"));
    };

    let mut struct_with: Vec<Path> = Vec::new();
    let mut messages: Option<Path> = None;
    let mut rename_all: Option<String> = None;
    for attr in &input.attrs {
        if attr.path().is_ident("validate") {
            attr.parse_nested_meta(|meta| {
                if meta.path.is_ident("with") {
                    struct_with.push(meta.value()?.parse()?);
                } else if meta.path.is_ident("messages") {
                    messages = Some(meta.value()?.parse()?);
                } else {
                    return Err(meta.error("expected `with = function` or `messages = function`"));
                }
                Ok(())
            })?;
        } else if attr.path().is_ident("serde") {
            let _ = attr.parse_nested_meta(|meta| {
                if meta.path.is_ident("rename_all") {
                    rename_all = Some(meta.value()?.parse::<LitStr>()?.value());
                } else if meta.input.peek(Token![=]) {
                    meta.value()?.parse::<Expr>()?;
                }
                Ok(())
            });
        }
    }

    let mut fields = Vec::new();
    for f in &named.named {
        let ident = f.ident.as_ref().expect("named field");
        let mut key = rename(&ident.to_string(), rename_all.as_deref());
        let mut rules = Vec::new();
        for attr in &f.attrs {
            if attr.path().is_ident("serde") {
                let _ = attr.parse_nested_meta(|meta| {
                    if meta.path.is_ident("rename") && meta.input.peek(Token![=]) {
                        key = meta.value()?.parse::<LitStr>()?.value();
                    } else if meta.input.peek(Token![=]) {
                        meta.value()?.parse::<Expr>()?;
                    }
                    Ok(())
                });
            } else if attr.path().is_ident("validate") {
                attr.parse_nested_meta(|meta| {
                    let name = meta.path.get_ident().map(Ident::to_string).unwrap_or_default();
                    let rule = match name.as_str() {
                        "required" => RuleAttr::Required,
                        // `Option<T>` already allows `None`; kept so attributes read like the rules table.
                        "nullable" => return Ok(()),
                        "email" | "url" | "uuid" | "date" | "integer" | "numeric" | "accepted" => {
                            let f = Ident::new(&name, Span::call_site());
                            RuleAttr::Rule(quote!(::validate::#f()))
                        }
                        "min" | "max" => {
                            let f = Ident::new(&name, Span::call_site());
                            let n: Expr = meta.value()?.parse()?;
                            RuleAttr::Rule(quote!(::validate::#f(#n)))
                        }
                        "between" | "one_of" => {
                            let content;
                            parenthesized!(content in meta.input);
                            let args = Punctuated::<Expr, Token![,]>::parse_terminated(&content)?;
                            let args: Vec<&Expr> = args.iter().collect();
                            match name.as_str() {
                                "between" if args.len() == 2 => {
                                    let (lo, hi) = (args[0], args[1]);
                                    RuleAttr::Rule(quote!(::validate::between(#lo, #hi)))
                                }
                                "between" => return Err(meta.error("expected `between(low, high)`")),
                                _ => RuleAttr::Rule(quote!(::validate::one_of([#(#args),*]))),
                            }
                        }
                        "same" => {
                            let other: Ident = meta.value()?.parse()?;
                            let other_field = named
                                .named
                                .iter()
                                .find(|f| f.ident.as_ref() == Some(&other))
                                .ok_or_else(|| Error::new_spanned(&other, "no such field"))?;
                            let value = if is_string(&other_field.ty) { quote!(self.#other.as_str()) } else { quote!(&self.#other) };
                            RuleAttr::Rule(quote!(::validate::same(#value)))
                        }
                        "rule" => {
                            let rule: Expr = meta.value()?.parse()?;
                            RuleAttr::Rule(quote!(#rule))
                        }
                        "with" => RuleAttr::With(meta.value()?.parse()?),
                        _ => {
                            return Err(meta.error(
                                "unknown rule; expected one of: required, nullable, min = n, max = n, between(a, b), \
                                 email, url, uuid, date, integer, numeric, one_of(..), same = field, accepted, \
                                 rule = MyRule, with = function",
                            ));
                        }
                    };
                    rules.push(rule);
                    Ok(())
                })?;
            }
        }
        if !rules.is_empty() {
            fields.push(FieldInfo { ident, ty: &f.ty, key, rules });
        }
    }

    let checks = fields.iter().map(field_checks);
    let struct_checks = struct_with.iter().map(|f| quote!(#f(self, e);));
    let messages = messages.map(|f| {
        quote! {
            fn messages(&self) -> ::validate::Messages {
                #f()
            }
        }
    });
    let name = &input.ident;
    let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();
    Ok(quote! {
        impl #impl_generics ::validate::Validate for #name #ty_generics #where_clause {
            fn validate(&self, e: &mut ::validate::Errors) {
                #(#checks)*
                #(#struct_checks)*
            }
            #messages
        }
    })
}

/// The rules of one field. `String` values are checked as `&str`; an
/// `Option` is checked when `Some`, and `None` fails only `required`.
fn field_checks(f: &FieldInfo) -> Tokens {
    let (ident, key) = (f.ident, &f.key);
    match option_inner(f.ty) {
        None => {
            let (value, value_ty) = view(quote!(self.#ident), f.ty, true);
            let rules = f.rules.iter().map(|r| rule_call(r, &value_ty));
            quote! { e.field(#key, #value)#(#rules)*; }
        }
        Some(inner) => {
            let required = f.rules.iter().any(|r| matches!(r, RuleAttr::Required));
            let (value, value_ty) = view(quote!(__value), inner, false);
            // `required` on the inner value only means something for text and lists.
            let keep_required = is_string(inner) || is_vec(inner);
            let rules = f.rules.iter().filter(|r| keep_required || !matches!(r, RuleAttr::Required)).map(|r| rule_call(r, &value_ty));
            let missing = if required { quote!(e.missing(#key);) } else { quote!() };
            quote! {
                match &self.#ident {
                    ::core::option::Option::Some(__value) => { e.field(#key, #value)#(#rules)*; }
                    ::core::option::Option::None => { #missing }
                }
            }
        }
    }
}

/// How a value is handed to the rules, and the type they see.
fn view(place: Tokens, ty: &Type, owned_place: bool) -> (Tokens, Tokens) {
    if is_string(ty) {
        (quote!(#place.as_str()), quote!(str))
    } else if owned_place {
        (quote!(&#place), quote!(#ty))
    } else {
        (quote!(#place), quote!(#ty))
    }
}

fn rule_call(rule: &RuleAttr, value_ty: &Tokens) -> Tokens {
    match rule {
        RuleAttr::Required => quote!(.rule(::validate::required())),
        RuleAttr::Rule(r) => quote!(.rule(#r)),
        RuleAttr::With(f) => {
            let name = f.segments.last().map(|s| s.ident.to_string()).unwrap_or_default();
            quote!(.rule(::validate::check_with(#name, |__v: &#value_ty| #f(self, __v))))
        }
    }
}

fn last_segment(ty: &Type) -> Option<&syn::PathSegment> {
    match ty {
        Type::Path(p) if p.qself.is_none() => p.path.segments.last(),
        _ => None,
    }
}

fn is_string(ty: &Type) -> bool {
    last_segment(ty).is_some_and(|s| s.ident == "String")
}

fn is_vec(ty: &Type) -> bool {
    last_segment(ty).is_some_and(|s| s.ident == "Vec")
}

fn option_inner(ty: &Type) -> Option<&Type> {
    let seg = last_segment(ty).filter(|s| s.ident == "Option")?;
    match &seg.arguments {
        syn::PathArguments::AngleBracketed(args) => match args.args.first() {
            Some(syn::GenericArgument::Type(t)) => Some(t),
            _ => None,
        },
        _ => None,
    }
}

/// The key serde uses for a field, under `#[serde(rename_all = "...")]`.
fn rename(field: &str, rule: Option<&str>) -> String {
    let words: Vec<&str> = field.split('_').filter(|w| !w.is_empty()).collect();
    let cap = |w: &str| {
        let mut c = w.chars();
        c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
    };
    match rule {
        Some("lowercase") => field.to_lowercase(),
        Some("UPPERCASE") => field.to_uppercase(),
        Some("PascalCase") => words.iter().map(|w| cap(w)).collect(),
        Some("camelCase") => {
            let pascal: String = words.iter().map(|w| cap(w)).collect();
            let mut c = pascal.chars();
            c.next().map(|f| f.to_lowercase().collect::<String>() + c.as_str()).unwrap_or_default()
        }
        Some("SCREAMING_SNAKE_CASE") => field.to_uppercase(),
        Some("kebab-case") => words.join("-"),
        Some("SCREAMING-KEBAB-CASE") => words.join("-").to_uppercase(),
        _ => field.to_owned(),
    }
}
