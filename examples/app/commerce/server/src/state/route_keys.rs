pub(crate) const HOME: &str = "home";
pub(crate) const SEARCH: &str = "search";
pub(crate) const CATEGORY: &str = "category";
pub(crate) const PRODUCT: &str = "product";

pub(crate) const ABOUT: &str = "about";
pub(crate) const TERMS: &str = "terms";
pub(crate) const SHIPPING: &str = "shipping";
pub(crate) const PRIVACY: &str = "privacy";
pub(crate) const FAQ: &str = "faq";

#[must_use]
pub(crate) fn is_static_page(route_key: &str) -> bool {
    matches!(route_key, ABOUT | TERMS | SHIPPING | PRIVACY | FAQ)
}
