use proc_macro::TokenStream;
use proc_macro2::{Span, TokenStream as TokenStream2};
use quote::{format_ident, quote, quote_spanned, ToTokens};
use std::collections::{HashMap, HashSet};
use syn::{
    parse::Parse, parse::ParseStream, parse_macro_input, FnArg, GenericArgument, ItemTrait,
    LitStr, PathArguments, ReturnType, Signature, Token, TraitItem, TraitItemFn, Type,
};

use crate::dialect;
use crate::dsl::{self, WhereChunk, WhereClause};

struct TableAttr {
    table: String,
    mock: bool,
    component: bool,
    joins: Vec<String>,
}

impl Parse for TableAttr {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let ident: syn::Ident = input.parse()?;
        if ident != "table" {
            return Err(syn::Error::new(ident.span(), "expected `table = \"name\"`"));
        }
        input.parse::<Token![=]>()?;
        let table = input.parse::<LitStr>()?.value();
        let mut mock = false;
        let mut component = false;
        let mut joins: Vec<String> = Vec::new();
        while input.peek(Token![,]) {
            input.parse::<Token![,]>()?;
            if input.is_empty() {
                break;
            }
            let flag: syn::Ident = input.parse()?;
            if flag == "mock" {
                mock = true;
            } else if flag == "component" {
                component = true;
            } else if flag == "join" {
                let content;
                syn::parenthesized!(content in input);
                loop {
                    let name: syn::Ident = content.parse()?;
                    joins.push(name.to_string());
                    if content.peek(Token![,]) {
                        content.parse::<Token![,]>()?;
                    } else {
                        break;
                    }
                }
            } else {
                return Err(syn::Error::new(
                    flag.span(),
                    format!("unknown attribute `{flag}`; expected `mock`, `component`, or `join(...)`"),
                ));
            }
        }
        Ok(TableAttr { table, mock, component, joins })
    }
}

struct Ctx<'a> {
    table: &'a str,
    joins: &'a [String],
}

pub fn expand(attr: TokenStream, item: TokenStream) -> TokenStream {
    let TableAttr { table, mock, component, joins } = parse_macro_input!(attr as TableAttr);
    let mut trait_def = parse_macro_input!(item as ItemTrait);
    let trait_name = trait_def.ident.clone();
    let vis = trait_def.vis.clone();
    let struct_name = format_ident!("Db{}", trait_name);
    let ctx = Ctx { table: &table, joins: &joins };

    let mut errors: Vec<TokenStream2> = Vec::new();
    let unique_map = collect_attr_map(&mut trait_def, "unique", &mut errors);
    let created_at_map = collect_attr_map(&mut trait_def, "created_at", &mut errors);
    let unchecked = collect_flag_attr(&mut trait_def, "unchecked_columns");
    let order_by_map = collect_order_by_map(&mut trait_def, &mut errors);
    validate_attr_targets(&unique_map, &created_at_map, &mut errors);

    let tx_parts = process_transactional(
        &mut trait_def,
        &ctx,
        &unique_map,
        &created_at_map,
        &unchecked,
        &order_by_map,
    );
    let (tx_trait_sigs, tx_impls, tx_probes): (Vec<_>, Vec<_>, Vec<_>) = itertools_unzip3(tx_parts);

    let mut method_impls: Vec<TokenStream2> = Vec::new();
    let mut probes: Vec<TokenStream2> = Vec::new();
    for item in &trait_def.items {
        if let TraitItem::Fn(m) = item {
            if m.default.is_none() {
                let (impl_tokens, probe_tokens) =
                    gen_method(m, &ctx, &unique_map, &created_at_map, &unchecked, &order_by_map);
                method_impls.push(impl_tokens);
                probes.push(probe_tokens);
            }
        }
    }

    let trait_attrs = &trait_def.attrs;
    let generics = &trait_def.generics;
    let colon_token = &trait_def.colon_token;
    let supertraits = &trait_def.supertraits;
    let trait_items = &trait_def.items;
    let mock_name = format_ident!("Mock{}", trait_name);
    let mock_attr = if mock {
        quote!(#[cfg_attr(test, ::mockall::automock)])
    } else {
        quote!()
    };
    let mock_repo_impl = if mock {
        quote! {
            #[cfg(test)]
            impl ::axumstart_db::Repository for #mock_name {
                fn pool(&self) -> &::axumstart_db::DbPool {
                    panic!("pool() called on mock")
                }
            }
        }
    } else {
        quote!()
    };

    let component_impls = if component {
        quote! {
            #[::axumstart_components::async_trait]
            impl ::axumstart_components::ComponentBlueprint for #struct_name {
                async fn new(ctx: &::axumstart_components::ComponentProvider) -> Self {
                    Self { pool: ctx.get_cloned::<::axumstart_db::DbPool>().await }
                }
            }
            impl ::axumstart_components::DynComponentBlueprint for #struct_name {
                type Dyn = dyn #trait_name;
                fn upcast(arc: ::std::sync::Arc<Self>) -> ::std::sync::Arc<dyn #trait_name> {
                    arc
                }
            }
            ::axumstart_components::inventory::submit! {
                ::axumstart_components::ComponentRegistration(|ctx: &::axumstart_components::ComponentProvider| {
                    ::axumstart_components::RegisterProbe::<#struct_name>::new().register(ctx);
                })
            }
        }
    } else {
        quote!()
    };

    TokenStream::from(quote! {
        #(#errors)*

        #(#trait_attrs)*
        #mock_attr
        #[::async_trait::async_trait]
        #vis trait #trait_name #generics #colon_token #supertraits {
            #(#trait_items)*
            #(#tx_trait_sigs;)*
        }

        #vis struct #struct_name {
            pool: ::axumstart_db::DbPool,
        }

        impl #struct_name {
            pub fn new(pool: ::axumstart_db::DbPool) -> Self {
                Self { pool }
            }
        }

        impl ::axumstart_db::Repository for #struct_name {
            fn pool(&self) -> &::axumstart_db::DbPool {
                &self.pool
            }
        }

        #[::async_trait::async_trait]
        impl #trait_name for #struct_name {
            #(#method_impls)*
            #(#tx_impls)*
        }

        #component_impls
        #mock_repo_impl

        #(#probes)*
        #(#tx_probes)*
    })
}

fn itertools_unzip3(
    v: Vec<(TokenStream2, TokenStream2, TokenStream2)>,
) -> (Vec<TokenStream2>, Vec<TokenStream2>, Vec<TokenStream2>) {
    let mut a = Vec::with_capacity(v.len());
    let mut b = Vec::with_capacity(v.len());
    let mut c = Vec::with_capacity(v.len());
    for (x, y, z) in v {
        a.push(x);
        b.push(y);
        c.push(z);
    }
    (a, b, c)
}

// Scans all trait methods for #[<attr_name>(col)], strips the attr in place,
// returns method_name → column map. Bad argument shape becomes a compile error.
fn collect_attr_map(
    trait_def: &mut ItemTrait,
    attr_name: &str,
    errors: &mut Vec<TokenStream2>,
) -> HashMap<String, (String, Span)> {
    let mut map = HashMap::new();
    for item in trait_def.items.iter_mut() {
        if let TraitItem::Fn(method) = item {
            if let Some(pos) = method.attrs.iter().position(|a| a.path().is_ident(attr_name)) {
                let attr = method.attrs.remove(pos);
                match attr.parse_args::<syn::Ident>() {
                    Ok(col) => {
                        map.insert(
                            method.sig.ident.to_string(),
                            (col.to_string(), method.sig.ident.span()),
                        );
                    }
                    Err(_) => {
                        let e = syn::Error::new(
                            method.sig.ident.span(),
                            format!("`#[{attr_name}(...)]` expects a single column identifier, e.g. `#[{attr_name}(user_id)]`"),
                        );
                        errors.push(e.to_compile_error());
                    }
                }
            }
        }
    }
    map
}

// Scans all trait methods for #[order_by("col1", "col2", ...)], strips the attr in
// place, and returns method_name → ordered column list. The method name must end in
// `_ordered`; codegen dispatches on the name with that suffix stripped.
fn collect_order_by_map(
    trait_def: &mut ItemTrait,
    errors: &mut Vec<TokenStream2>,
) -> HashMap<String, Vec<String>> {
    let mut map = HashMap::new();
    for item in trait_def.items.iter_mut() {
        let TraitItem::Fn(method) = item else { continue };
        let Some(pos) = method.attrs.iter().position(|a| a.path().is_ident("order_by")) else {
            continue;
        };
        let attr = method.attrs.remove(pos);
        let name = method.sig.ident.to_string();
        let span = method.sig.ident.span();

        if !name.ends_with("_ordered") {
            errors.push(
                syn::Error::new(
                    span,
                    format!(
                        "`#[order_by(...)]` on `{name}` requires the method name to end with `_ordered`"
                    ),
                )
                .to_compile_error(),
            );
            continue;
        }

        match attr.parse_args_with(|input: ParseStream| {
            let mut cols = Vec::new();
            loop {
                cols.push(input.parse::<LitStr>()?.value());
                if input.peek(Token![,]) {
                    input.parse::<Token![,]>()?;
                } else {
                    break;
                }
            }
            Ok(cols)
        }) {
            Ok(cols) => {
                map.insert(name, cols);
            }
            Err(_) => {
                errors.push(
                    syn::Error::new(
                        span,
                        "`#[order_by(...)]` expects one or more string column names, e.g. `#[order_by(\"col1\", \"col2\")]`",
                    )
                    .to_compile_error(),
                );
            }
        }
    }
    map
}

// Strips a bare marker attribute (e.g. #[unchecked_columns]) from methods,
// returning the names of methods that carried it.
fn collect_flag_attr(trait_def: &mut ItemTrait, attr_name: &str) -> HashSet<String> {
    let mut set = HashSet::new();
    for item in trait_def.items.iter_mut() {
        if let TraitItem::Fn(method) = item {
            if let Some(pos) = method.attrs.iter().position(|a| a.path().is_ident(attr_name)) {
                method.attrs.remove(pos);
                set.insert(method.sig.ident.to_string());
            }
        }
    }
    set
}

fn validate_attr_targets(
    unique_map: &HashMap<String, (String, Span)>,
    created_at_map: &HashMap<String, (String, Span)>,
    errors: &mut Vec<TokenStream2>,
) {
    for (method, (_, span)) in unique_map {
        if method != "upsert" {
            errors.push(
                syn::Error::new(
                    *span,
                    format!("`#[unique(...)]` on `{method}` has no effect; it only applies to `upsert`"),
                )
                .to_compile_error(),
            );
        }
    }
    for (method, (_, span)) in created_at_map {
        if !method.contains("_this_week") {
            errors.push(
                syn::Error::new(
                    *span,
                    format!("`#[created_at(...)]` on `{method}` has no effect; it only applies to `*_this_week` methods"),
                )
                .to_compile_error(),
            );
        }
    }
}

fn process_transactional(
    trait_def: &mut ItemTrait,
    ctx: &Ctx,
    unique_map: &HashMap<String, (String, Span)>,
    created_at_map: &HashMap<String, (String, Span)>,
    unchecked: &HashSet<String>,
    order_by_map: &HashMap<String, Vec<String>>,
) -> Vec<(TokenStream2, TokenStream2, TokenStream2)> {
    let mut orig_sigs: Vec<Signature> = Vec::new();

    for item in trait_def.items.iter_mut() {
        if let TraitItem::Fn(method) = item {
            if let Some(pos) = method.attrs.iter().position(|a| a.path().is_ident("transactional")) {
                method.attrs.remove(pos);
                orig_sigs.push(method.sig.clone());
            }
        }
    }

    orig_sigs
        .into_iter()
        .map(|sig| {
            let tx_sig = make_tx_sig(&sig);
            let name = sig.ident.to_string();
            let unique_col = unique_map.get(&name).map(|(c, _)| c.as_str());
            let date_col = created_at_map.get(&name).map(|(c, _)| c.as_str());
            let order_cols = order_by_map.get(&name).map(|c| c.as_slice());
            match gen_body(&name, &sig, ctx, quote!(conn), unique_col, date_col, order_cols) {
                Ok((body, probe)) => {
                    let probe = if unchecked.contains(&name) { quote!() } else { probe };
                    (quote!(#tx_sig), quote!(#tx_sig { #body }), probe)
                }
                Err(e) => {
                    let err = e.to_compile_error();
                    (quote!(#tx_sig), quote!(#tx_sig { #err }), quote!())
                }
            }
        })
        .collect()
}

fn make_tx_sig(sig: &Signature) -> Signature {
    let mut tx_sig = sig.clone();
    tx_sig.ident = format_ident!("{}_tx", sig.ident);
    let conn: FnArg = syn::parse_quote!(conn: &mut ::axumstart_db::DbConnection);
    tx_sig.inputs.insert(1, conn);
    tx_sig
}

fn gen_method(
    m: &TraitItemFn,
    ctx: &Ctx,
    unique_map: &HashMap<String, (String, Span)>,
    created_at_map: &HashMap<String, (String, Span)>,
    unchecked: &HashSet<String>,
    order_by_map: &HashMap<String, Vec<String>>,
) -> (TokenStream2, TokenStream2) {
    let sig = &m.sig;
    let name = sig.ident.to_string();
    let unique_col = unique_map.get(&name).map(|(c, _)| c.as_str());
    let date_col = created_at_map.get(&name).map(|(c, _)| c.as_str());
    let order_cols = order_by_map.get(&name).map(|c| c.as_slice());
    match gen_body(&name, sig, ctx, quote!(::axumstart_db::Repository::pool(self)), unique_col, date_col, order_cols) {
        Ok((body, probe)) => {
            let probe = if unchecked.contains(&name) { quote!() } else { probe };
            (quote! { #sig { #body } }, probe)
        }
        Err(e) => {
            let err = e.to_compile_error();
            (quote! { #sig { #err } }, quote!())
        }
    }
}

fn no_order_by(sig: &Signature, order_cols: Option<&[String]>, name: &str) -> syn::Result<()> {
    if order_cols.is_some() {
        return Err(syn::Error::new(
            sig.ident.span(),
            format!(
                "`#[order_by(...)]` has no effect on `{name}`; only supported on find_by_*, \
                 find_all_by_*, find_all, and bare boolean find_all_<field> methods"
            ),
        ));
    }
    Ok(())
}

fn gen_body(
    name: &str,
    sig: &Signature,
    ctx: &Ctx,
    exec: TokenStream2,
    unique_col: Option<&str>,
    date_col: Option<&str>,
    order_cols: Option<&[String]>,
) -> syn::Result<(TokenStream2, TokenStream2)> {
    let dispatch_name = match order_cols {
        Some(_) => name.strip_suffix("_ordered").unwrap_or(name),
        None => name,
    };

    if let Some(f) = dispatch_name.strip_prefix("find_all_by_") {
        gen_select_all(sig, ctx, f, exec, date_col, order_cols)
    } else if let Some(f) = dispatch_name.strip_prefix("find_random_by_") {
        gen_filtered(sig, ctx, f, exec, Kind::Random, order_cols)
    } else if let Some(f) = dispatch_name.strip_prefix("find_by_") {
        gen_filtered(sig, ctx, f, exec, Kind::One, order_cols)
    } else if let Some(f) = dispatch_name.strip_prefix("count_by_") {
        gen_filtered(sig, ctx, f, exec, Kind::Count, order_cols)
    } else if let Some(f) = dispatch_name.strip_prefix("exists_by_") {
        gen_filtered(sig, ctx, f, exec, Kind::Exists, order_cols)
    } else if let Some(f) = dispatch_name.strip_prefix("delete_by_") {
        gen_filtered(sig, ctx, f, exec, Kind::Delete, order_cols)
    } else if dispatch_name == "find_all" {
        gen_select_all_no_filter(sig, ctx, exec, None, order_cols)
    } else if let Some(order_str) = dispatch_name.strip_prefix("find_all_order_by_") {
        no_order_by(sig, order_cols, name)?;
        gen_select_all_no_filter(sig, ctx, exec, Some(order_str), None)
    } else if let Some(field) = dispatch_name.strip_prefix("find_all_") {
        gen_select_all_bool_flag(sig, ctx, field, exec, order_cols)
    } else if dispatch_name == "upsert" {
        no_order_by(sig, order_cols, name)?;
        gen_upsert(sig, ctx, exec, unique_col)
    } else if dispatch_name == "update" {
        no_order_by(sig, order_cols, name)?;
        gen_delegated(sig, ctx, exec, "update", quote!(__sqlx_update_in))
    } else if dispatch_name == "insert" {
        no_order_by(sig, order_cols, name)?;
        gen_delegated(sig, ctx, exec, "insert", quote!(__sqlx_insert_into))
    } else if dispatch_name == "insert_or_ignore" {
        no_order_by(sig, order_cols, name)?;
        gen_delegated(sig, ctx, exec, "insert_or_ignore", quote!(__sqlx_insert_ignore_into))
    } else if dispatch_name == "insert_all" {
        no_order_by(sig, order_cols, name)?;
        gen_insert_all(sig, ctx, exec)
    } else if let Some(rest) = dispatch_name.strip_prefix("set_") {
        no_order_by(sig, order_cols, name)?;
        gen_set(sig, ctx, rest, exec)
    } else if dispatch_name == "pool" {
        no_order_by(sig, order_cols, name)?;
        Ok((quote! { ::axumstart_db::Repository::pool(self) }, quote!()))
    } else {
        Err(syn::Error::new(
            sig.ident.span(),
            format!(
                "no codegen rule for `{name}`; expected one of: find_by_*, find_all, \
                 find_all_by_*, find_all_order_by_*, find_random_by_*, find_all_<bool_field>, \
                 count_by_*, exists_by_*, delete_by_*, set_*_by_*, insert, insert_all, \
                 insert_or_ignore, upsert, update, pool — or provide a method body"
            ),
        ))
    }
}

// ---------- parameter handling ----------

struct SplitParams {
    /// Non-Page value parameters, in declaration order.
    binds: Vec<TokenStream2>,
    /// Pattern of the `Page`-typed parameter, if any.
    page: Option<TokenStream2>,
}

fn split_params(sig: &Signature) -> syn::Result<SplitParams> {
    let mut binds = Vec::new();
    let mut page = None;
    for arg in &sig.inputs {
        if let FnArg::Typed(pt) = arg {
            if is_page_type(&pt.ty) {
                if page.is_some() {
                    return Err(syn::Error::new(
                        sig.ident.span(),
                        "at most one `Page` parameter is allowed",
                    ));
                }
                page = Some(pt.pat.to_token_stream());
            } else {
                binds.push(pt.pat.to_token_stream());
            }
        }
    }
    Ok(SplitParams { binds, page })
}

fn is_page_type(ty: &Type) -> bool {
    matches!(ty, Type::Path(tp) if tp.path.segments.last().is_some_and(|s| s.ident == "Page"))
}

fn check_arity(sig: &Signature, actual: usize, expected: usize) -> syn::Result<()> {
    if actual != expected {
        return Err(syn::Error::new(
            sig.ident.span(),
            format!(
                "`{}` takes {actual} bindable parameter(s) but its name implies {expected} \
                 (excluding `&self` and any `Page` parameter)",
                sig.ident
            ),
        ));
    }
    Ok(())
}

fn no_page(sig: &Signature, sp: &SplitParams) -> syn::Result<()> {
    if sp.page.is_some() {
        return Err(syn::Error::new(
            sig.ident.span(),
            "`Page` parameter is only supported on find_all* methods",
        ));
    }
    Ok(())
}

// ---------- return type analysis ----------

fn generic_arg(ty: &Type, seg_name: &str) -> Option<Type> {
    let Type::Path(tp) = ty else { return None };
    let seg = tp.path.segments.last()?;
    if seg.ident != seg_name {
        return None;
    }
    let PathArguments::AngleBracketed(ab) = &seg.arguments else { return None };
    ab.args.iter().find_map(|a| match a {
        GenericArgument::Type(t) => Some(t.clone()),
        _ => None,
    })
}

fn result_ok_type(ret: &ReturnType) -> Option<Type> {
    let ReturnType::Type(_, ty) = ret else { return None };
    generic_arg(ty, "Result")
}

fn fetch_method(ret: &ReturnType) -> TokenStream2 {
    if let Some(ok) = result_ok_type(ret) {
        if generic_arg(&ok, "Option").is_some() {
            return quote!(fetch_optional);
        }
        if generic_arg(&ok, "Vec").is_some() {
            return quote!(fetch_all);
        }
    }
    quote!(fetch_one)
}

/// Row type for column probing: Result<Row>, Result<Option<Row>>, Result<Vec<Row>>.
/// Only path types qualify — tuples and scalars are skipped by the callers' kind gating.
fn infer_row_type(ret: &ReturnType) -> Option<Type> {
    let ok = result_ok_type(ret)?;
    let inner = generic_arg(&ok, "Option")
        .or_else(|| generic_arg(&ok, "Vec"))
        .unwrap_or(ok);
    matches!(inner, Type::Path(_)).then_some(inner)
}

// Generates a compile-time check that every DSL column exists as a field on the
// row/values struct. Spanned to the method name so field typos point at the method.
fn make_probe(span: Span, ty: &Type, cols: &[String]) -> TokenStream2 {
    let fields: Vec<syn::Ident> = cols
        .iter()
        .filter_map(|c| syn::parse_str::<syn::Ident>(c).ok())
        .map(|mut id| {
            id.set_span(span);
            id
        })
        .collect();
    if fields.is_empty() {
        return quote!();
    }
    quote_spanned! {span=>
        const _: () = {
            #[allow(dead_code)]
            fn _column_check(r: &#ty) {
                #( let _ = &r.#fields; )*
            }
        };
    }
}

fn row_probe(sig: &Signature, cols: &[String]) -> TokenStream2 {
    if cols.is_empty() {
        return quote!();
    }
    match infer_row_type(&sig.output) {
        Some(ty) => make_probe(sig.ident.span(), &ty, cols),
        None => quote!(),
    }
}

// Generates a compile-time check that `field` on the row struct is actually `bool`-typed,
// not just present. Used for the `find_all_<field>` bare boolean filter, where a
// non-bool column would otherwise only fail at query time as a type mismatch.
fn make_bool_probe(span: Span, ty: &Type, field: &str) -> TokenStream2 {
    let Ok(mut ident) = syn::parse_str::<syn::Ident>(field) else { return quote!() };
    ident.set_span(span);
    quote_spanned! {span=>
        const _: () = {
            #[allow(dead_code)]
            fn _bool_flag_check(r: &#ty) {
                let _: bool = r.#ident;
            }
        };
    }
}

// ---------- QueryBuilder codegen for the `_in` DSL suffix ----------

/// Renders each `WhereChunk` as a statement pushing onto a live `__qb: QueryBuilder`.
/// `binds` must be the same bind-expression slice whose indices `chunks` was built
/// against (see `dsl::build_where`'s `bind_offset` parameter).
fn chunk_statements(chunks: &[WhereChunk], binds: &[TokenStream2]) -> Vec<TokenStream2> {
    chunks
        .iter()
        .map(|c| match c {
            WhereChunk::Literal(s) => quote!(__qb.push(#s);),
            WhereChunk::Bind(i) => {
                let b = &binds[*i];
                quote!(__qb.push_bind(#b);)
            }
            WhereChunk::InList(i) => {
                let b = &binds[*i];
                quote!(::axumstart_db::push_in_list(&mut __qb, #b);)
            }
        })
        .collect()
}

// ---------- query generators ----------

enum Kind {
    One,
    Random,
    Count,
    Exists,
    Delete,
}

fn gen_filtered(
    sig: &Signature,
    ctx: &Ctx,
    field_str: &str,
    exec: TokenStream2,
    kind: Kind,
    order_cols: Option<&[String]>,
) -> syn::Result<(TokenStream2, TokenStream2)> {
    if order_cols.is_some() && !matches!(kind, Kind::One) {
        return Err(syn::Error::new(
            sig.ident.span(),
            "`#[order_by(...)]` is only supported on `find_by_*` (not find_random_by_/count_by_/exists_by_/delete_by_)",
        ));
    }
    let conds = dsl::parse_conditions(field_str)
        .map_err(|e| syn::Error::new(sig.ident.span(), e))?;
    let wc: WhereClause = dsl::build_where(&conds, ctx.joins, 0, 0, &dialect::placeholder);
    let sp = split_params(sig)?;
    no_page(sig, &sp)?;
    check_arity(sig, sp.binds.len(), wc.params)?;

    let table = ctx.table;
    let join_sql = if wc.joins_needed.is_empty() {
        String::new()
    } else {
        format!(" {}", dsl::join_clauses(table, &wc.joins_needed))
    };
    let qualified = if wc.joins_needed.is_empty() { "*".to_string() } else { format!("\"{table}\".*") };
    let mut probe_cols = wc.probe_cols.clone();
    if let (Kind::One, Some(cols)) = (&kind, order_cols) {
        probe_cols.extend(cols.iter().cloned());
    }

    let binds = &sp.binds;
    let stmts = chunk_statements(&wc.chunks, binds);

    let body = match kind {
        Kind::One => {
            let sql_prefix = format!("SELECT {qualified} FROM \"{table}\"{join_sql} WHERE ");
            let fetch = fetch_method(&sig.output);
            if wc.has_in {
                let order_stmt = order_cols.map(|cols| {
                    let s = format!(" ORDER BY {}", cols.join(", "));
                    quote!(__qb.push(#s);)
                });
                quote! {
                    {
                        let mut __qb = ::sqlx::QueryBuilder::<::axumstart_db::Db>::new(#sql_prefix);
                        #(#stmts)*
                        #order_stmt
                        __qb.build_query_as().#fetch(#exec).await
                    }
                }
            } else {
                let mut sql = format!("{sql_prefix}{}", wc.sql);
                if let Some(cols) = order_cols {
                    sql.push_str(" ORDER BY ");
                    sql.push_str(&cols.join(", "));
                }
                quote! {
                    ::sqlx::query_as(#sql)
                        #(.bind(#binds))*
                        .#fetch(#exec)
                        .await
                }
            }
        }
        Kind::Random => {
            let sql_prefix = format!("SELECT {qualified} FROM \"{table}\"{join_sql} WHERE ");
            let fetch = fetch_method(&sig.output);
            let random_sql = dialect::random_order_sql();
            if wc.has_in {
                quote! {
                    {
                        let mut __qb = ::sqlx::QueryBuilder::<::axumstart_db::Db>::new(#sql_prefix);
                        #(#stmts)*
                        __qb.push(#random_sql);
                        __qb.push(" LIMIT 1");
                        __qb.build_query_as().#fetch(#exec).await
                    }
                }
            } else {
                let sql = format!("{sql_prefix}{} {random_sql} LIMIT 1", wc.sql);
                quote! {
                    ::sqlx::query_as(#sql)
                        #(.bind(#binds))*
                        .#fetch(#exec)
                        .await
                }
            }
        }
        Kind::Count => {
            let sql_prefix = format!("SELECT COUNT(*) FROM \"{table}\"{join_sql} WHERE ");
            if wc.has_in {
                quote! {
                    {
                        let mut __qb = ::sqlx::QueryBuilder::<::axumstart_db::Db>::new(#sql_prefix);
                        #(#stmts)*
                        __qb.build_query_scalar().fetch_one(#exec).await
                    }
                }
            } else {
                let sql = format!("{sql_prefix}{}", wc.sql);
                quote! {
                    ::sqlx::query_scalar(#sql)
                        #(.bind(#binds))*
                        .fetch_one(#exec)
                        .await
                }
            }
        }
        Kind::Exists => {
            let sql_prefix = format!("SELECT EXISTS(SELECT 1 FROM \"{table}\"{join_sql} WHERE ");
            if wc.has_in {
                quote! {
                    {
                        let mut __qb = ::sqlx::QueryBuilder::<::axumstart_db::Db>::new(#sql_prefix);
                        #(#stmts)*
                        __qb.push(")");
                        __qb.build_query_scalar().fetch_one(#exec).await
                    }
                }
            } else {
                let sql = format!("{sql_prefix}{})", wc.sql);
                quote! {
                    ::sqlx::query_scalar(#sql)
                        #(.bind(#binds))*
                        .fetch_one(#exec)
                        .await
                }
            }
        }
        Kind::Delete => {
            if !wc.joins_needed.is_empty() {
                return Err(syn::Error::new(
                    sig.ident.span(),
                    "`delete_by_*` cannot filter through joined tables; use the FK column or a method body",
                ));
            }
            let sql_prefix = format!("DELETE FROM \"{table}\" WHERE ");
            if wc.has_in {
                quote! {
                    {
                        let mut __qb = ::sqlx::QueryBuilder::<::axumstart_db::Db>::new(#sql_prefix);
                        #(#stmts)*
                        __qb.build().execute(#exec).await.map(|_| ())
                    }
                }
            } else {
                let sql = format!("{sql_prefix}{}", wc.sql);
                quote! {
                    ::sqlx::query(#sql)
                        #(.bind(#binds))*
                        .execute(#exec)
                        .await
                        .map(|_| ())
                }
            }
        }
    };

    // Only find* methods return the row type — probing count/exists/delete would
    // wrongly probe i64/bool/().
    let probe = match kind {
        Kind::One | Kind::Random => row_probe(sig, &probe_cols),
        _ => quote!(),
    };
    Ok((body, probe))
}

fn gen_select_all(
    sig: &Signature,
    ctx: &Ctx,
    field_str: &str,
    exec: TokenStream2,
    date_col: Option<&str>,
    order_cols: Option<&[String]>,
) -> syn::Result<(TokenStream2, TokenStream2)> {
    let (after_order, order) = dsl::split_order(field_str);
    if order.is_some() && order_cols.is_some() {
        return Err(syn::Error::new(
            sig.ident.span(),
            "cannot combine an embedded `_order_by_` in the method name with `#[order_by(...)]`; use one or the other",
        ));
    }
    let (filter_str, this_week) = dsl::split_this_week(after_order);
    let conds = dsl::parse_conditions(filter_str)
        .map_err(|e| syn::Error::new(sig.ident.span(), e))?;
    let wc: WhereClause = dsl::build_where(&conds, ctx.joins, 0, 0, &dialect::placeholder);
    let sp = split_params(sig)?;
    check_arity(sig, sp.binds.len(), wc.params)?;

    let table = ctx.table;
    let mut probe_cols = wc.probe_cols.clone();
    let date_col_name = date_col.unwrap_or("created_at").to_string();
    if this_week {
        probe_cols.push(date_col_name.clone());
    }

    let select_prefix = if wc.joins_needed.is_empty() {
        format!("SELECT * FROM \"{table}\" WHERE ")
    } else {
        format!("SELECT \"{table}\".* FROM \"{table}\" {} WHERE ", dsl::join_clauses(table, &wc.joins_needed))
    };

    let order_sql: Option<String> = if let Some((order_sql, order_col)) = &order {
        probe_cols.push(order_col.clone());
        Some(format!(" {order_sql}"))
    } else if let Some(cols) = order_cols {
        probe_cols.extend(cols.iter().cloned());
        Some(format!(" ORDER BY {}", cols.join(", ")))
    } else {
        None
    };

    let binds = &sp.binds;

    let body = if wc.has_in {
        let stmts = chunk_statements(&wc.chunks, binds);
        let week_stmt = this_week.then(|| {
            let lit = format!(" AND {date_col_name} >= ");
            quote!(__qb.push(#lit); __qb.push(::axumstart_db::WEEK_START_SQL);)
        });
        let order_stmt = order_sql.as_ref().map(|o| quote!(__qb.push(#o);));
        let page_stmt = sp.page.as_ref().map(|page| {
            quote! {
                __qb.push(" LIMIT ");
                __qb.push_bind(#page.limit);
                __qb.push(" OFFSET ");
                __qb.push_bind(#page.offset);
            }
        });
        quote! {
            {
                let mut __qb = ::sqlx::QueryBuilder::<::axumstart_db::Db>::new(#select_prefix);
                #(#stmts)*
                #week_stmt
                #order_stmt
                #page_stmt
                __qb.build_query_as().fetch_all(#exec).await
            }
        }
    } else if this_week {
        let date_filter_template = format!(" AND {date_col_name} >= {{}}");
        let mut sql_template = format!("{select_prefix}{}{date_filter_template}", wc.sql);
        if let Some(o) = &order_sql {
            sql_template.push_str(o);
        }
        let page_binds = append_page(&mut sql_template, &sp, wc.params);
        quote! {
            {
                static __SQL: ::std::sync::OnceLock<::std::string::String> = ::std::sync::OnceLock::new();
                let sql: &str = __SQL
                    .get_or_init(|| ::std::format!(#sql_template, ::axumstart_db::WEEK_START_SQL))
                    .as_str();
                ::sqlx::query_as(sql)
                    #(.bind(#binds))*
                    #page_binds
                    .fetch_all(#exec)
                    .await
            }
        }
    } else {
        let mut sql = format!("{select_prefix}{}", wc.sql);
        if let Some(o) = &order_sql {
            sql.push_str(o);
        }
        let page_binds = append_page(&mut sql, &sp, wc.params);
        quote! {
            ::sqlx::query_as(#sql)
                #(.bind(#binds))*
                #page_binds
                .fetch_all(#exec)
                .await
        }
    };

    let probe = row_probe(sig, &probe_cols);
    Ok((body, probe))
}

fn gen_select_all_no_filter(
    sig: &Signature,
    ctx: &Ctx,
    exec: TokenStream2,
    order_str: Option<&str>,
    order_cols: Option<&[String]>,
) -> syn::Result<(TokenStream2, TokenStream2)> {
    let sp = split_params(sig)?;
    check_arity(sig, sp.binds.len(), 0)?;
    let table = ctx.table;
    let mut probe_cols: Vec<String> = Vec::new();
    let mut sql = match order_str {
        Some(o) => {
            let (rendered, col) = dsl::format_order_col(o);
            probe_cols.push(col);
            format!("SELECT * FROM \"{table}\" ORDER BY {rendered}")
        }
        None => format!("SELECT * FROM \"{table}\""),
    };
    if let Some(cols) = order_cols {
        sql.push_str(" ORDER BY ");
        sql.push_str(&cols.join(", "));
        probe_cols.extend(cols.iter().cloned());
    }
    let page_binds = append_page(&mut sql, &sp, 0);
    let fetch = fetch_method(&sig.output);
    let probe = row_probe(sig, &probe_cols);
    Ok((
        quote! {
            ::sqlx::query_as(#sql)
                #page_binds
                .#fetch(#exec)
                .await
        },
        probe,
    ))
}

// find_all_<field> (no `_by_`) — bare boolean flag filter, no bound parameter: `WHERE field = TRUE`.
// The extra bool-typed probe catches the case where `field` exists on the Row but isn't a bool
// column, which `row_probe`'s existence-only check wouldn't.
fn gen_select_all_bool_flag(
    sig: &Signature,
    ctx: &Ctx,
    field: &str,
    exec: TokenStream2,
    order_cols: Option<&[String]>,
) -> syn::Result<(TokenStream2, TokenStream2)> {
    let sp = split_params(sig)?;
    check_arity(sig, sp.binds.len(), 0)?;
    let table = ctx.table;
    let mut probe_cols = vec![field.to_string()];
    let mut sql = format!("SELECT * FROM \"{table}\" WHERE {field} = TRUE");
    if let Some(cols) = order_cols {
        sql.push_str(" ORDER BY ");
        sql.push_str(&cols.join(", "));
        probe_cols.extend(cols.iter().cloned());
    }
    let page_binds = append_page(&mut sql, &sp, 0);
    let fetch = fetch_method(&sig.output);

    let mut probe = row_probe(sig, &probe_cols);
    if let Some(ty) = infer_row_type(&sig.output) {
        let bool_probe = make_bool_probe(sig.ident.span(), &ty, field);
        probe = quote! { #probe #bool_probe };
    }

    Ok((
        quote! {
            ::sqlx::query_as(#sql)
                #page_binds
                .#fetch(#exec)
                .await
        },
        probe,
    ))
}

/// Appends `LIMIT $n OFFSET $m` (dialect-correct placeholders) and returns the extra
/// bind calls, if a Page param exists.
fn append_page(sql: &mut String, sp: &SplitParams, params_before: usize) -> TokenStream2 {
    match &sp.page {
        Some(page) => {
            let limit_idx = params_before + 1;
            let offset_idx = params_before + 2;
            let limit_ph = dialect::placeholder(limit_idx);
            let offset_ph = dialect::placeholder(offset_idx);
            sql.push_str(&format!(" LIMIT {limit_ph} OFFSET {offset_ph}"));
            quote!(.bind(#page.limit).bind(#page.offset))
        }
        None => quote!(),
    }
}

// set_{field}_by_{filter} — first (n - filter_count) params are SET values, rest are WHERE values.
// e.g. set_email_verified_at_by_id(&self, email_verified_at: DateTime, id: i32)
//      → UPDATE "{table}" SET email_verified_at = $1 WHERE id = $2
fn gen_set(
    sig: &Signature,
    ctx: &Ctx,
    rest: &str,
    exec: TokenStream2,
) -> syn::Result<(TokenStream2, TokenStream2)> {
    let by_pos = rest.find("_by_").ok_or_else(|| {
        syn::Error::new(
            sig.ident.span(),
            "`set_*` methods require a `_by_` filter (e.g. `set_email_by_id`) — or provide a method body",
        )
    })?;
    let field = &rest[..by_pos];
    let filter_str = &rest[by_pos + 4..];

    let sp = split_params(sig)?;
    no_page(sig, &sp)?;
    let set_cols: Vec<&str> = field.split("_and_").collect();
    let filter_conds = dsl::parse_conditions(filter_str)
        .map_err(|e| syn::Error::new(sig.ident.span(), e))?;
    let wc: WhereClause =
        dsl::build_where(&filter_conds, ctx.joins, set_cols.len(), set_cols.len(), &dialect::placeholder);
    check_arity(sig, sp.binds.len(), set_cols.len() + wc.params)?;

    let binds = &sp.binds;
    let set_params = &sp.binds[..set_cols.len()];
    let filter_params = &sp.binds[set_cols.len()..];
    let table = ctx.table;

    let body = if wc.has_in {
        let where_stmts = chunk_statements(&wc.chunks, binds);
        let mut set_stmts: Vec<TokenStream2> = Vec::new();
        for (i, col) in set_cols.iter().enumerate() {
            if i > 0 {
                set_stmts.push(quote!(__qb.push(", ");));
            }
            let lit = format!("{col} = ");
            let b = &set_params[i];
            set_stmts.push(quote!(__qb.push(#lit); __qb.push_bind(#b);));
        }
        let set_prefix = format!("UPDATE \"{table}\" SET ");
        quote! {
            {
                let mut __qb = ::sqlx::QueryBuilder::<::axumstart_db::Db>::new(#set_prefix);
                #(#set_stmts)*
                __qb.push(" WHERE ");
                #(#where_stmts)*
                __qb.build().execute(#exec).await.map(|_| ())
            }
        }
    } else {
        let set_clause = set_cols
            .iter()
            .enumerate()
            .map(|(i, col)| format!("{col} = {}", dialect::placeholder(i + 1)))
            .collect::<Vec<_>>()
            .join(", ");
        let sql = format!("UPDATE \"{table}\" SET {set_clause} WHERE {}", wc.sql);
        quote! {
            ::sqlx::query(#sql)
                #(.bind(#set_params))*
                #(.bind(#filter_params))*
                .execute(#exec)
                .await
                .map(|_| ())
        }
    };

    Ok((body, quote!()))
}

/// First non-receiver parameter (pattern and type) — errors if absent.
fn first_value_param(sig: &Signature, what: &str) -> syn::Result<(TokenStream2, Type)> {
    sig.inputs
        .iter()
        .find_map(|arg| match arg {
            FnArg::Typed(pt) => {
                let p = &pt.pat;
                Some((quote!(#p), (*pt.ty).clone()))
            }
            FnArg::Receiver(_) => None,
        })
        .ok_or_else(|| {
            syn::Error::new(sig.ident.span(), format!("`{what}` requires a values parameter"))
        })
}

fn gen_upsert(
    sig: &Signature,
    ctx: &Ctx,
    exec: TokenStream2,
    unique_col: Option<&str>,
) -> syn::Result<(TokenStream2, TokenStream2)> {
    let conflict_col = unique_col.ok_or_else(|| {
        syn::Error::new(
            sig.ident.span(),
            "`upsert` requires `#[unique(col)]` on the method to name the conflict column",
        )
    })?;
    let (values, values_ty) = first_value_param(sig, "upsert")?;
    let table = ctx.table;
    // Conflict column must be a field of the values struct — probe it too.
    let probe = make_probe(sig.ident.span(), &values_ty, &[conflict_col.to_string()]);
    Ok((
        quote! { #values.__sqlx_upsert_into(#table, #conflict_col, #exec).await },
        probe,
    ))
}

fn gen_delegated(
    sig: &Signature,
    ctx: &Ctx,
    exec: TokenStream2,
    what: &str,
    method: TokenStream2,
) -> syn::Result<(TokenStream2, TokenStream2)> {
    let (values, _) = first_value_param(sig, what)?;
    let table = ctx.table;
    Ok((quote! { #values.#method(#table, #exec).await }, quote!()))
}

fn gen_insert_all(
    sig: &Signature,
    ctx: &Ctx,
    exec: TokenStream2,
) -> syn::Result<(TokenStream2, TokenStream2)> {
    let (values, values_ty) = first_value_param(sig, "insert_all")?;
    let elem_ty = generic_arg(&values_ty, "Vec").ok_or_else(|| {
        syn::Error::new(
            sig.ident.span(),
            "`insert_all` requires a `Vec<T>` values parameter where T derives SqlxInsert",
        )
    })?;
    let table = ctx.table;
    Ok((
        quote! { <#elem_ty>::__sqlx_insert_all_into(#values, #table, #exec).await },
        quote!(),
    ))
}
