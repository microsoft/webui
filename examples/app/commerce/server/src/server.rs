// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use actix_web::http::header::LOCATION;
use actix_web::{web, HttpRequest, HttpResponse};
use serde_json::Value;

use crate::app::AppState;
use crate::cart::{self, build_cart_state, clear_cookie, cookie_for_cart};
use crate::catalog::Catalog;
use crate::error::ServerError;
use crate::extractors::{CartMutationInput, CartMutationPayload, RequestContext};
use crate::security;
use crate::state;

struct CartResponseOptions<'a> {
    should_reset: bool,
    cart: cart::StoredCart,
    redirect_to: Option<&'a str>,
    open_cart: bool,
}

pub(crate) fn configure_app(cfg: &mut web::ServiceConfig) {
    cfg.route(
        "/_image/{stem}",
        web::get().to(crate::image_proxy::serve_image),
    );
    cfg.service(web::scope("/cart").configure(configure_cart_routes));
    configure_frontend_routes(cfg);
}

fn configure_cart_routes(cfg: &mut web::ServiceConfig) {
    cfg.route("/add", web::post().to(add_to_cart))
        .route("/update", web::post().to(update_cart));
}

fn configure_frontend_routes(cfg: &mut web::ServiceConfig) {
    cfg.default_service(web::route().to(handle_frontend_request));
}

async fn handle_frontend_request(
    context: RequestContext,
    data: web::Data<AppState>,
) -> Result<HttpResponse, ServerError> {
    if let Some(relative) = context.asset_path() {
        if let Some(response) = data.frontend().serve_asset(relative) {
            return Ok(response);
        }
    }

    let route_params = data.frontend().collect_route_params(context.route_path());
    let stable_path = cart::without_cart(context.request_path());
    let cart_state = build_cart_state(&context.cart_read().cart, data.catalog(), &stable_path);
    let is_partial = context.wants_json();
    let (page_state, image_preloads) = state::build_route_state(&state::RouteStateRequest {
        catalog: data.catalog(),
        route_path: context.route_path(),
        params: &route_params,
        request_path: context.request_path(),
        cart_state: &cart_state,
        is_partial,
    })
    .ok_or(ServerError::NotFound)?;

    if context.wants_json() {
        return Ok(partial_response(&context, data.get_ref(), &page_state));
    }

    let nonce = security::generate_nonce();
    let html = data
        .frontend()
        .render_html(context.route_path(), &page_state, &nonce)
        .map_err(ServerError::RenderFailed)?;
    Ok(html_response(
        &context,
        inject_head_preload_tags(html, &image_preloads),
        &nonce,
    ))
}

async fn add_to_cart(
    req: HttpRequest,
    context: RequestContext,
    payload: CartMutationPayload,
    data: web::Data<AppState>,
) -> Result<HttpResponse, ServerError> {
    if !security::passes_csrf_check(&req) {
        return Err(ServerError::CsrfRejected);
    }
    if !data.rate_limiter().check(security::client_ip(&req)) {
        return Err(ServerError::RateLimited);
    }
    let wants_json = context.wants_json();
    let mut cart_read = context.into_cart_read();
    let input = cart_mutation_input(payload);
    let product = data
        .catalog()
        .by_handle(&input.handle)
        .ok_or(ServerError::UnknownProduct)?;

    let quantity = input.quantity.unwrap_or(1).clamp(1, 99);
    let (color, size) = cart::resolve_variant(
        product,
        input.color.as_deref().unwrap_or_default(),
        input.size.as_deref().unwrap_or_default(),
    );
    cart::add_item(&mut cart_read.cart, &input.handle, &color, &size, quantity);

    Ok(cart_response(
        wants_json,
        data.catalog(),
        CartResponseOptions {
            should_reset: cart_read.should_reset,
            cart: cart_read.cart,
            redirect_to: input.redirect_to.as_deref(),
            open_cart: input.open_cart.unwrap_or(true),
        },
    ))
}

async fn update_cart(
    req: HttpRequest,
    context: RequestContext,
    payload: CartMutationPayload,
    data: web::Data<AppState>,
) -> Result<HttpResponse, ServerError> {
    if !security::passes_csrf_check(&req) {
        return Err(ServerError::CsrfRejected);
    }
    if !data.rate_limiter().check(security::client_ip(&req)) {
        return Err(ServerError::RateLimited);
    }
    let wants_json = context.wants_json();
    let mut cart_read = context.into_cart_read();
    let input = cart_mutation_input(payload);
    let quantity = input.quantity.unwrap_or(0).min(99);

    cart::update_item(
        &mut cart_read.cart,
        &input.handle,
        input.color.as_deref().unwrap_or_default(),
        input.size.as_deref().unwrap_or_default(),
        quantity,
    );

    Ok(cart_response(
        wants_json,
        data.catalog(),
        CartResponseOptions {
            should_reset: cart_read.should_reset,
            cart: cart_read.cart,
            redirect_to: input.redirect_to.as_deref(),
            open_cart: input.open_cart.unwrap_or(true),
        },
    ))
}

fn cart_mutation_input(payload: CartMutationPayload) -> CartMutationInput {
    match payload {
        actix_web::Either::Left(form) => form.into_inner(),
        actix_web::Either::Right(json) => json.into_inner(),
    }
}

fn partial_response(
    context: &RequestContext,
    state: &AppState,
    page_state: &Value,
) -> HttpResponse {
    let payload = state.frontend().render_partial(
        context.route_path(),
        context.request_path(),
        context.inventory_hex(),
        page_state.clone(),
    );

    let mut builder = HttpResponse::Ok();
    builder.content_type("application/json");
    builder.insert_header(("Cache-Control", "private, no-store"));
    builder.insert_header(("Vary", "Accept, Cookie"));
    if context.cart_read().should_reset {
        builder.cookie(clear_cookie());
    }
    builder.json(payload)
}

fn html_response(context: &RequestContext, html: String, nonce: &str) -> HttpResponse {
    let mut builder = HttpResponse::Ok();
    builder.content_type("text/html; charset=utf-8");
    builder.insert_header(("Cache-Control", "private, no-store"));
    builder.insert_header(("Vary", "Accept, Cookie"));
    builder.insert_header(("Content-Security-Policy", security::csp_header(nonce)));
    if context.cart_read().should_reset {
        builder.cookie(clear_cookie());
    }
    builder.body(html)
}

fn inject_head_preload_tags(mut html: String, image_urls: &[String]) -> String {
    let Some(head_end) = html.find("</head>") else {
        return html;
    };

    let preloads = build_head_preload_tags(image_urls);
    if preloads.is_empty() {
        return html;
    }

    html.insert_str(head_end, &preloads);
    html
}

/// Build SSR-only `<link rel="preload">` tags for images and scripts.
/// CSS preloads are handled by the framework via protocol strategy fields —
/// no custom logic needed here.
/// The router removes these managed tags on SPA navigations so preloads never
/// leak across routes.
fn build_head_preload_tags(image_urls: &[String]) -> String {
    let capacity = 80 + image_urls.len() * 120;
    let mut buf = String::with_capacity(capacity);

    buf.push_str(r#"<link rel="modulepreload" href="/index.js" data-webui-ssr-preload="script">"#);

    for (i, url) in image_urls.iter().enumerate() {
        buf.push_str(r#"<link rel="preload" as="image" href=""#);
        buf.push_str(url);
        if i == 0 {
            buf.push_str(r#"" fetchpriority="high" data-webui-ssr-preload="image">"#);
        } else {
            buf.push_str(r#"" data-webui-ssr-preload="image">"#);
        }
    }

    buf
}

fn cart_response(
    wants_json: bool,
    catalog: &Catalog,
    options: CartResponseOptions<'_>,
) -> HttpResponse {
    let stable_path = cart::sanitize_redirect(options.redirect_to);
    let payload = state::cart_state_payload(
        &build_cart_state(&options.cart, catalog, &stable_path),
        &stable_path,
        options.open_cart,
    );

    let mut builder = if wants_json {
        HttpResponse::Ok()
    } else {
        let location = if options.open_cart {
            cart::with_cart_open(&stable_path, true)
        } else {
            stable_path
        };
        let mut redirect = HttpResponse::SeeOther();
        redirect.insert_header((LOCATION, location));
        redirect
    };

    if options.should_reset && options.cart.lines.is_empty() {
        builder.cookie(clear_cookie());
    } else if let Some(cookie) = cookie_for_cart(&options.cart) {
        builder.cookie(cookie);
    } else {
        builder.cookie(clear_cookie());
    }

    builder.insert_header(("Cache-Control", "private, no-store"));
    builder.insert_header(("Vary", "Accept, Cookie"));

    if wants_json {
        builder.content_type("application/json");
        builder.json(payload)
    } else {
        builder.finish()
    }
}

#[cfg(test)]
mod tests {
    use super::configure_app;
    use crate::app::{test_state, test_state_with_css};
    use crate::cart;
    use actix_web::body::to_bytes;
    use actix_web::http::{header, StatusCode};
    use actix_web::test::{self, TestRequest};
    use actix_web::App;

    #[actix_web::test]
    async fn search_route_renders_html_from_direct_server() {
        let app =
            test::init_service(App::new().app_data(test_state()).configure(configure_app)).await;

        let request = TestRequest::with_uri("/search/shirts").to_request();
        let response = test::call_service(&app, request).await;

        assert_eq!(response.status(), StatusCode::OK);
        assert!(
            response.headers().get(header::LINK).is_none(),
            "SSR preload tags should be emitted in <head>, not the HTTP Link header"
        );
        let body = match to_bytes(response.into_body()).await {
            Ok(body) => body,
            Err(error) => panic!("{error}"),
        };
        let html = match String::from_utf8(body.to_vec()) {
            Ok(html) => html,
            Err(error) => panic!("{error}"),
        };

        assert!(html.contains("mp-page-search"));
        assert!(html.contains("mp-navbar"));
        // CSS preloads are now emitted by the framework (via protocol strategy
        // not the custom server. Verify the framework-emitted preload is present.
        assert!(
            html.contains(r#"data-webui-ssr-preload="style""#),
            "Framework should emit CSS preload with data-webui-ssr-preload: {html}"
        );
        assert!(html.contains(r#"href="/_image/t-shirt-1?w=640&q=75""#));
        assert!(
            !html.contains(r#"\"data-webui-ssr-preload\""#),
            "server-only preload tags should not leak into serialized client state"
        );
    }

    #[actix_web::test]
    async fn search_without_category_renders_product_cards() {
        let app =
            test::init_service(App::new().app_data(test_state()).configure(configure_app)).await;

        let request = TestRequest::with_uri("/search").to_request();
        let response = test::call_service(&app, request).await;

        assert_eq!(response.status(), StatusCode::OK);
        let body = match to_bytes(response.into_body()).await {
            Ok(body) => body,
            Err(error) => panic!("{error}"),
        };
        let html = match String::from_utf8(body.to_vec()) {
            Ok(html) => html,
            Err(error) => panic!("{error}"),
        };

        assert!(
            html.contains("mp-product-card"),
            "SSR for /search must render product cards in the outlet"
        );
    }

    #[actix_web::test]
    async fn category_partial_excludes_shell_state() {
        let app =
            test::init_service(App::new().app_data(test_state()).configure(configure_app)).await;

        let request = TestRequest::with_uri("/search/shirts")
            .insert_header((header::ACCEPT, "application/json"))
            .to_request();
        let response = test::call_service(&app, request).await;

        assert_eq!(response.status(), StatusCode::OK);
        let body = match to_bytes(response.into_body()).await {
            Ok(body) => body,
            Err(error) => panic!("{error}"),
        };
        let json: serde_json::Value = match serde_json::from_slice(&body) {
            Ok(json) => json,
            Err(error) => panic!("{error}"),
        };

        // State is at top level (caller adds it), not per-entry in chain
        assert!(json.get("state").is_some(), "should have top-level state");
        assert!(json["state"].get("products").is_some());
        assert!(json["state"].get("categories").is_some());
        assert!(json["state"].get("sortOptions").is_some());

        // Shell state excluded from page-specific state
        assert!(json["state"].get("storeName").is_none());
        assert!(json["state"].get("cartItems").is_none());
        assert!(json["state"].get("cartItemCount").is_none());
        assert!(json["state"].get("page").is_none());
        assert!(json["state"].get("head_end").is_none());
    }

    #[actix_web::test]
    async fn category_partial_in_module_mode_splits_styles_from_templates() {
        let app = test::init_service(
            App::new()
                .app_data(test_state_with_css(webui::CssStrategy::Module))
                .configure(configure_app),
        )
        .await;

        let request = TestRequest::with_uri("/search/shirts")
            .insert_header((header::ACCEPT, "application/json"))
            .to_request();
        let response = test::call_service(&app, request).await;

        assert_eq!(response.status(), StatusCode::OK);
        let body = match to_bytes(response.into_body()).await {
            Ok(body) => body,
            Err(error) => panic!("{error}"),
        };
        let json: serde_json::Value = match serde_json::from_slice(&body) {
            Ok(json) => json,
            Err(error) => panic!("{error}"),
        };

        let template_styles = match json["templateStyles"].as_array() {
            Some(template_styles) => template_styles,
            None => panic!("templateStyles should be an array"),
        };
        let templates = match json["templates"].as_array() {
            Some(templates) => templates,
            None => panic!("templates should be an array"),
        };
        let combined_styles = template_styles
            .iter()
            .filter_map(serde_json::Value::as_str)
            .collect::<String>();
        let combined_templates = templates
            .iter()
            .filter_map(serde_json::Value::as_str)
            .collect::<String>();

        assert!(
            combined_styles.contains(r#"<style type="module" specifier="mp-product-grid">"#),
            "module partials should return module CSS definitions separately: {combined_styles}"
        );
        assert!(
            !combined_templates.contains(r#"<style type="module" specifier="mp-product-grid">"#),
            "template scripts should not be prefixed with module styles: {combined_templates}"
        );
        assert!(
            !combined_templates.contains(r#"<link rel="stylesheet" href="/mp-product-grid.css""#),
            "module partials must not ship link-mode product-grid templates: {combined_templates}"
        );
        assert!(
            !combined_templates.contains(r#"<link rel="stylesheet" href="/mp-page-search.css""#),
            "module partials must not ship link-mode page-search templates: {combined_templates}"
        );
    }

    #[actix_web::test]
    async fn module_mode_about_page_includes_cart_panel_style() {
        let state = test_state_with_css(webui::CssStrategy::Module);
        let app = test::init_service(App::new().app_data(state).configure(configure_app)).await;

        let request = TestRequest::with_uri("/about").to_request();
        let response = test::call_service(&app, request).await;
        assert_eq!(response.status(), StatusCode::OK);

        let body = match to_bytes(response.into_body()).await {
            Ok(body) => body,
            Err(error) => panic!("{error}"),
        };
        let html = match String::from_utf8(body.to_vec()) {
            Ok(html) => html,
            Err(error) => panic!("{error}"),
        };

        // mp-cart-panel is a non-route sibling inside mp-app whose FNV-1a hash
        // collides with mp-app (both map to bit 218). The inventory filter must
        // not drop it due to this collision. The style is emitted inline in the
        // component's light DOM during SSR rendering.
        assert!(
            html.contains(r#"<style type="module" specifier="mp-cart-panel">"#),
            "mp-cart-panel module style should be present in SSR output for /about"
        );
    }

    #[actix_web::test]
    async fn search_with_query_renders_empty_results_state() {
        let app =
            test::init_service(App::new().app_data(test_state()).configure(configure_app)).await;

        let request = TestRequest::with_uri("/search?q=bottle").to_request();
        let response = test::call_service(&app, request).await;

        assert_eq!(response.status(), StatusCode::OK);
        let body = match to_bytes(response.into_body()).await {
            Ok(body) => body,
            Err(error) => panic!("{error}"),
        };
        let html = match String::from_utf8(body.to_vec()) {
            Ok(html) => html,
            Err(error) => panic!("{error}"),
        };

        assert!(html.contains(r#"query="bottle""#));
        assert!(html.contains("There are no products that match"));
        assert!(html.contains(">bottle<"));
    }

    #[actix_web::test]
    async fn cart_add_sets_cookie_and_returns_shell_state() {
        let app =
            test::init_service(App::new().app_data(test_state()).configure(configure_app)).await;

        let request = TestRequest::post()
            .uri("/cart/add")
            .insert_header((header::ACCEPT, "application/json"))
            .set_json(serde_json::json!({
                "handle": "acme-t-shirt",
                "color": "Black",
                "size": "M",
                "redirectTo": "/product/acme-t-shirt",
                "openCart": true
            }))
            .to_request();
        let response = test::call_service(&app, request).await;

        assert_eq!(response.status(), StatusCode::OK);
        let cookies: Vec<_> = response.response().cookies().collect();
        assert!(
            cookies
                .iter()
                .any(|cookie| cookie.name() == cart::CART_COOKIE_NAME),
            "expected cart cookie to be set"
        );

        let body = match to_bytes(response.into_body()).await {
            Ok(body) => body,
            Err(error) => panic!("{error}"),
        };
        let json: serde_json::Value = match serde_json::from_slice(&body) {
            Ok(json) => json,
            Err(error) => panic!("{error}"),
        };

        assert_eq!(json["cartItemCount"], 1);
        assert_eq!(json["cartOpen"], "true");
    }

    #[actix_web::test]
    async fn cart_add_rejects_unknown_product() {
        let app =
            test::init_service(App::new().app_data(test_state()).configure(configure_app)).await;

        let request = TestRequest::post()
            .uri("/cart/add")
            .set_json(serde_json::json!({
                "handle": "missing-product"
            }))
            .to_request();
        let response = test::call_service(&app, request).await;

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[actix_web::test]
    async fn image_proxy_serves_cached_image() {
        let state = test_state();
        let expected = state
            .image_cache()
            .get("baby-cap-white", 384)
            .unwrap_or_else(|| panic!("expected cached baby-cap-white image"));
        let app = test::init_service(App::new().app_data(state).configure(configure_app)).await;

        let request = TestRequest::get()
            .uri("/_image/baby-cap-white?w=256&q=75")
            .to_request();
        let response = test::call_service(&app, request).await;

        assert_eq!(response.status(), StatusCode::OK);
        let content_type = response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        assert_eq!(content_type, "image/avif");

        let cache_control = response
            .headers()
            .get("cache-control")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        assert!(cache_control.contains("immutable"));

        let body = match to_bytes(response.into_body()).await {
            Ok(body) => body,
            Err(error) => panic!("{error}"),
        };
        assert_eq!(
            body.as_ref(),
            expected.as_ref(),
            "proxy response should return the exact cached bytes for the snapped width"
        );
    }

    #[actix_web::test]
    async fn image_proxy_returns_404_for_unknown_image() {
        let app =
            test::init_service(App::new().app_data(test_state()).configure(configure_app)).await;

        let request = TestRequest::get()
            .uri("/_image/no-such-image?w=96&q=75")
            .to_request();
        let response = test::call_service(&app, request).await;

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }
}
