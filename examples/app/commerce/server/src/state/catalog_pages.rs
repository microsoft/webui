// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use serde_json::Value;

use crate::catalog::{self, Product};

use super::context::ShellContext;
use super::shell::{apply_page_metadata, page_state_base};

pub(crate) fn home_state(context: &ShellContext<'_>, is_partial: bool) -> Value {
    let mut state = page_state_base(context, is_partial);
    if !is_partial {
        apply_page_metadata(&mut state, "home", false, "default-shell");
    }
    state.insert(
        "featuredProducts".into(),
        Value::Array(catalog::products_to_json(&context.catalog.home_featured())),
    );
    state.insert(
        "carouselProducts".into(),
        Value::Array(catalog::products_to_json(&context.catalog.home_carousel())),
    );
    Value::Object(state)
}

pub(crate) fn search_state(
    context: &ShellContext<'_>,
    query: &str,
    requested_sort: Option<&str>,
    is_partial: bool,
) -> Value {
    let sort = default_sort(requested_sort);
    let products = if query.is_empty() {
        context.catalog.all()
    } else {
        context.catalog.search(query)
    };
    let products = catalog::sorted(products, sort);

    let mut state = page_state_base(context, is_partial);
    if !is_partial {
        apply_page_metadata(&mut state, "search", true, "catalog-shell");
    }
    state.insert(
        "products".into(),
        Value::Array(catalog::products_to_json(&products)),
    );
    state.insert(
        "categories".into(),
        Value::Array(catalog::categories_with_active_json(
            context.catalog.categories(),
            "",
        )),
    );
    state.insert(
        "sortOptions".into(),
        Value::Array(catalog::sort_options_json(sort, "/search", query)),
    );
    state.insert("allActiveClass".into(), Value::String("active".to_string()));
    state.insert(
        "currentCategoryLabel".into(),
        Value::String("All".to_string()),
    );
    state.insert("query".into(), Value::String(query.to_string()));
    state.insert("searchQuery".into(), Value::String(query.to_string()));
    state.insert("resultsCount".into(), serde_json::json!(products.len()));
    state.insert("activeCategory".into(), Value::String(String::new()));
    Value::Object(state)
}

pub(crate) fn category_state(
    context: &ShellContext<'_>,
    category: &str,
    query: &str,
    requested_sort: Option<&str>,
    is_partial: bool,
) -> Option<Value> {
    if !context
        .catalog
        .categories()
        .iter()
        .any(|item| item.handle == category)
    {
        return None;
    }

    let sort = default_sort(requested_sort);
    let products = if query.is_empty() {
        context.catalog.by_category(category)
    } else {
        context.catalog.search_in_category(category, query)
    };
    let products = catalog::sorted(products, sort);

    let mut state = page_state_base(context, is_partial);
    if !is_partial {
        apply_page_metadata(&mut state, "category", true, "catalog-shell");
    }
    state.insert(
        "products".into(),
        Value::Array(catalog::products_to_json(&products)),
    );
    state.insert(
        "categories".into(),
        Value::Array(catalog::categories_with_active_json(
            context.catalog.categories(),
            category,
        )),
    );
    state.insert(
        "currentCategoryLabel".into(),
        Value::String(active_category_title(context, category).to_string()),
    );
    state.insert(
        "sortOptions".into(),
        Value::Array(catalog::sort_options_json(
            sort,
            &category_search_path(category),
            query,
        )),
    );
    state.insert("allActiveClass".into(), Value::String(String::new()));
    state.insert("query".into(), Value::String(query.to_string()));
    state.insert("searchQuery".into(), Value::String(query.to_string()));
    state.insert("resultsCount".into(), serde_json::json!(products.len()));
    state.insert("activeCategory".into(), Value::String(category.to_string()));
    Some(Value::Object(state))
}

pub(crate) fn product_state(
    context: &ShellContext<'_>,
    handle: &str,
    is_partial: bool,
) -> Option<Value> {
    let product = context.catalog.by_handle(handle)?;
    let related = context.catalog.related(handle, 10);

    let mut state = page_state_base(context, is_partial);
    if !is_partial {
        apply_page_metadata(&mut state, "product", false, "default-shell");
    }
    state.insert(
        "relatedProducts".into(),
        Value::Array(catalog::products_to_json(&related)),
    );
    state.insert(
        "categories".into(),
        Value::Array(catalog::categories_with_active_json(
            context.catalog.categories(),
            active_category(product),
        )),
    );
    catalog::extend_product_detail_state(&mut state, product);

    Some(Value::Object(state))
}

fn active_category(product: &Product) -> &str {
    product.category.as_str()
}

fn active_category_title<'a>(context: &'a ShellContext<'_>, category: &'a str) -> &'a str {
    context
        .catalog
        .categories()
        .iter()
        .find(|item| item.handle == category)
        .map_or(category, |item| item.title.as_str())
}

fn default_sort(requested_sort: Option<&str>) -> &str {
    match requested_sort {
        Some(sort) if !sort.is_empty() => sort,
        _ => "relevance",
    }
}

fn category_search_path(category: &str) -> String {
    let mut path = String::with_capacity("/search/".len() + category.len());
    path.push_str("/search/");
    path.push_str(category);
    path
}
