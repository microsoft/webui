use serde_json::{Map, Value};

use crate::cart::{self, CartState};

use super::context::{ShellContext, STORE_NAME};

pub(crate) fn cart_state_payload(
    cart_state: &CartState,
    stable_path: &str,
    open_cart: bool,
) -> Value {
    let mut payload = Map::new();
    merge_cart_state(&mut payload, cart_state, stable_path, open_cart);
    Value::Object(payload)
}

pub(crate) fn base_state(context: &ShellContext<'_>) -> Map<String, Value> {
    let mut state = Map::new();
    state.insert("storeName".into(), Value::String(STORE_NAME.to_string()));
    state.insert(
        "currentCategoryLabel".into(),
        Value::String("All".to_string()),
    );
    state.insert(
        "navCategories".into(),
        Value::Array(
            context
                .catalog
                .top_nav_categories()
                .into_iter()
                .map(|category| {
                    serde_json::json!({
                        "handle": category.handle,
                        "title": category.title,
                    })
                })
                .collect(),
        ),
    );
    merge_cart_state(
        &mut state,
        context.cart_state,
        &context.stable_path,
        context.cart_open,
    );
    state
}

pub(crate) fn apply_page_metadata(
    state: &mut Map<String, Value>,
    page: &str,
    show_catalog_nav: bool,
    shell_class: &str,
) {
    state.insert("page".into(), Value::String(page.to_string()));
    state.insert(
        "showCatalogNav".into(),
        Value::String(if show_catalog_nav { "true" } else { "" }.to_string()),
    );
    state.insert("shellClass".into(), Value::String(shell_class.to_string()));
}

fn merge_cart_state(
    state: &mut Map<String, Value>,
    cart_state: &CartState,
    stable_path: &str,
    cart_open: bool,
) {
    state.insert(
        "cartItemCount".into(),
        serde_json::json!(cart_state.cart_item_count),
    );
    state.insert("cartItems".into(), serde_json::json!(cart_state.cart_items));
    state.insert("cartEmpty".into(), Value::Bool(cart_state.cart_empty));
    state.insert(
        "cartSubtotal".into(),
        Value::String(cart_state.cart_subtotal.clone()),
    );
    state.insert(
        "cartTaxes".into(),
        Value::String(cart_state.cart_taxes.clone()),
    );
    state.insert("currentPath".into(), Value::String(stable_path.to_string()));
    state.insert(
        "cartOpen".into(),
        Value::String(if cart_open { "true" } else { "" }.to_string()),
    );
    state.insert(
        "cartHref".into(),
        Value::String(cart::with_cart_open(stable_path, true)),
    );
    state.insert(
        "cartCloseHref".into(),
        Value::String(stable_path.to_string()),
    );
}
