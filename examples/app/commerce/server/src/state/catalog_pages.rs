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
    let featured = context.catalog.home_featured();
    if !is_partial {
        let urls: Vec<&str> = featured.iter().map(|p| p.image_url.as_str()).collect();
        state.insert("head_end".into(), Value::String(build_preload_tags(&urls)));
    }
    state.insert(
        "featuredProducts".into(),
        Value::Array(catalog::products_to_json(&featured)),
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
        if let Some(first) = products.first() {
            state.insert(
                "head_end".into(),
                Value::String(build_preload_tags(&[&first.image_url])),
            );
        }
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
    state.insert("allActive".into(), Value::Bool(true));
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
        if let Some(first) = products.first() {
            state.insert(
                "head_end".into(),
                Value::String(build_preload_tags(&[&first.image_url])),
            );
        }
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
    state.insert("allActive".into(), Value::Bool(false));
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
        state.insert(
            "head_end".into(),
            Value::String(build_preload_tags(&[&product.image_url])),
        );
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

/// Build `<link rel="preload">` tags for above-the-fold images so the browser
/// can start fetching them in parallel with CSS and JS during the initial HTML
/// parse, improving LCP. Only the first image gets `fetchpriority="high"` to
/// avoid competing with critical CSS/JS resources.
fn build_preload_tags(image_urls: &[&str]) -> String {
    const HIGH_PREFIX: &str = "<link rel=\"preload\" as=\"image\" fetchpriority=\"high\" href=\"";
    const NORMAL_PREFIX: &str = "<link rel=\"preload\" as=\"image\" href=\"";
    const TAG_SUFFIX: &str = "\">";
    let estimate = image_urls.len() * (HIGH_PREFIX.len() + TAG_SUFFIX.len() + 120);
    let mut buf = String::with_capacity(estimate);
    for (i, url) in image_urls.iter().enumerate() {
        let prefix = if i == 0 { HIGH_PREFIX } else { NORMAL_PREFIX };
        buf.push_str(prefix);
        buf.push_str(&html_escape::encode_double_quoted_attribute(url));
        buf.push_str(TAG_SUFFIX);
    }
    buf
}

#[cfg(test)]
mod tests {
    use super::build_preload_tags;

    #[test]
    fn preload_tags_first_image_gets_high_priority() {
        let tags = build_preload_tags(&["https://cdn.example.com/a.png", "https://cdn.example.com/b.png"]);
        assert!(
            tags.contains("fetchpriority=\"high\""),
            "first image should have fetchpriority=high"
        );
        assert!(
            tags.contains("https://cdn.example.com/a.png"),
            "first image URL should be present"
        );
        // Second image should NOT have fetchpriority
        assert!(tags.contains(r#"<link rel="preload" as="image" href="https://cdn.example.com/b.png">"#));
        assert_eq!(
            tags.matches("fetchpriority").count(),
            1,
            "only the first image should have fetchpriority"
        );
    }

    #[test]
    fn preload_tags_escapes_html_in_urls() {
        let tags = build_preload_tags(&["https://cdn.example.com/img?a=1&b=2"]);
        assert!(
            tags.contains("a=1&amp;b=2"),
            "ampersand in URL should be HTML-escaped"
        );
    }

    #[test]
    fn preload_tags_empty_urls_returns_empty() {
        assert!(build_preload_tags(&[]).is_empty());
    }

    #[test]
    fn preload_tags_single_url_gets_high_priority() {
        let tags = build_preload_tags(&["https://cdn.example.com/hero.png"]);
        assert!(tags.contains("fetchpriority=\"high\""));
        assert!(tags.contains("hero.png"));
    }
}
