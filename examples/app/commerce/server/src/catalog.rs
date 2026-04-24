// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

#![allow(clippy::disallowed_methods)]

use serde::Serialize;
use serde_json::{Map, Value};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Product {
    pub handle: String,
    pub title: String,
    pub description: String,
    pub description_html: String,
    pub price: String,
    pub price_raw: f64,
    pub compare_at: String,
    pub currency: String,
    pub category: String,
    pub gradient: String,
    pub gradient_alt: String,
    pub image_url: String,
    pub image_alt_url: String,
    pub gallery_image_urls: Vec<String>,
    pub available: bool,
    pub tags: Vec<String>,
    pub colors: Vec<VariantOption>,
    pub sizes: Vec<VariantOption>,
    collections: Vec<String>,
}

#[cfg(test)]
impl Product {
    fn belongs_to(&self, category: &str) -> bool {
        self.collections
            .iter()
            .any(|collection| collection == category)
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct VariantOption {
    pub value: String,
    pub available: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct Category {
    pub handle: String,
    pub title: String,
    pub count: usize,
}

struct ProductDef {
    handle: &'static str,
    title: &'static str,
    description: &'static str,
    description_html: &'static str,
    image_url: &'static str,
    gallery_image_urls: &'static [&'static str],
    price: f64,
    collections: &'static [&'static str],
}

struct CategoryDef {
    handle: &'static str,
    title: &'static str,
}

const PRODUCT_DEFS: &[ProductDef] = &[
    ProductDef {
        handle: "acme-t-shirt",
        title: "Acme T-Shirt",
        description: "60% combed ringspun cotton/40% polyester jersey tee.",
        description_html: r#"<p>60% combed ringspun cotton/40% polyester jersey tee.</p>"#,
        image_url: "t-shirt-color-black",
        gallery_image_urls: &[
            "t-shirt-color-black",
            "t-shirt-color-blue",
            "t-shirt-color-gray",
            "t-shirt-color-pink",
            "t-shirt-color-white",
        ],
        price: 20.00,
        collections: &[
            "shirts",
        ],
    },

    ProductDef {
        handle: "acme-rainbow-prism-t-shirt",
        title: "Acme Prism T-Shirt",
        description: "60% combed ringspun cotton/40% polyester jersey tee.",
        description_html: r#"<p>60% combed ringspun cotton/40% polyester jersey tee.</p>"#,
        image_url: "t-shirt-spiral-1",
        gallery_image_urls: &[
            "t-shirt-spiral-1",
            "t-shirt-spiral-2",
            "t-shirt-spiral-3",
            "t-shirt-spiral-4",
        ],
        price: 25.00,
        collections: &[
            "shirts",
        ],
    },

    ProductDef {
        handle: "acme-geometric-circles-t-shirt",
        title: "Acme Circles T-Shirt",
        description: "60% combed ringspun cotton/40% polyester jersey tee.",
        description_html: r#"<p>60% combed ringspun cotton/40% polyester jersey tee.</p>"#,
        image_url: "t-shirt-1",
        gallery_image_urls: &[
            "t-shirt-1",
            "t-shirt-2",
            "t-shirt-circles-blue",
        ],
        price: 20.00,
        collections: &[
            "shirts",
        ],
    },

    ProductDef {
        handle: "acme-drawstring-bag",
        title: "Acme Drawstring Bag",
        description: "Strong 210D ripstop nylon drawstring bag. Available in multiple sizes. Easy-to-close durable drawstring. Sturdy, reusable, and resilient.",
        description_html: r#"<ul>
<li>
<span style="font-size: 0.875rem;">Strong 210D ripstop nylon drawstring bag</span><br>
</li>
<li>
<span style="font-size: 0.875rem;">Available in multiple sizes</span><br>
</li>
<li>
<span style="font-size: 0.875rem;">Easy-to-close durable drawstring</span><br>
</li>
<li>
<span style="font-size: 0.875rem;">Sturdy, reusable, and resilient</span><br>
</li>
</ul>"#,
        image_url: "bag-1-dark",
        gallery_image_urls: &[
            "bag-1-dark",
            "bag-1-light",
        ],
        price: 12.00,
        collections: &[
            "bags",
        ],
    },

    ProductDef {
        handle: "acme-cup",
        title: "Acme Cup",
        description: "12oz double wall ceramic body with a padded bottom.",
        description_html: r#"<p>12oz double wall ceramic body with a padded bottom.</p>"#,
        image_url: "cup-black",
        gallery_image_urls: &[
            "cup-black",
            "cup-white",
        ],
        price: 15.00,
        collections: &[
            "drinkware",
        ],
    },

    ProductDef {
        handle: "acme-mug",
        title: "Acme Mug",
        description: "12 oz Beck Cork-Bottom Mug.",
        description_html: r#"<p>12 oz Beck Cork-Bottom Mug.</p>"#,
        image_url: "mug-1",
        gallery_image_urls: &[
            "mug-1",
            "mug-2",
        ],
        price: 15.00,
        collections: &[
            "drinkware",
        ],
    },

    ProductDef {
        handle: "acme-mechanical-keyboard",
        title: "Acme Keyboard",
        description: "",
        description_html: "",
        image_url: "keyboard",
        gallery_image_urls: &[
            "keyboard",
        ],
        price: 150.00,
        collections: &[
            "electronics",
        ],
    },

    ProductDef {
        handle: "acme-slip-on-shoes",
        title: "Acme Slip-On Shoes",
        description: r#"Step into summer! Every time your head looks down, you'll see these beauties, and your mood bounces right back up. Sleek, easy, and effortlessly stylish. Acme Slip-On Shoes are the ultimate get-up-and-go footwear. The low-profile slip-on canvas upper offers unbeatable convenience, while the clean design makes this all-white slip-on the perfect choice for anyone with places to go and things to do. One of the most popular designs, these shoes are the perfect middle ground between style and convenience."#,
        description_html: r#"<p>Step into summer! Every time your head looks&nbsp;down, you'll see these beauties, and your mood bounces right back up.</p>
<p>Sleek, easy, and effortlessly stylish. Acme&nbsp;Slip-On Shoes are the ultimate get-up-and-go footwear. The low-profile slip-on canvas upper offers unbeatable convenience, while the clean design makes this all-white slip-on the perfect choice for anyone with places to go and things to do. One of the most popular designs, these shoes are the perfect middle ground between style and convenience.</p>
<ul>
<li>
<span style="font-size: 0.875rem;">Iconic slip-on shoe</span><br>
</li>
<li>
<span style="font-size: 0.875rem;">Low profile canvas uppers</span><br>
</li>
<li>
<span style="font-size: 0.875rem;">Supportive padded collars</span><br>
</li>
<li>
<span style="font-size: 0.875rem;">Elastic side accents</span><br>
</li>
<li>
<span style="font-size: 0.875rem;">Signature rubber waffle outsoles</span><br>
</li>
</ul>"#,
        image_url: "shoes-1",
        gallery_image_urls: &[
            "shoes-1",
            "shoes-2",
            "shoes-3",
            "shoes-4",
        ],
        price: 45.00,
        collections: &[
            "footware",
        ],
    },

    ProductDef {
        handle: "acme-baby-cap",
        title: "Acme Baby Cap",
        description: "100% combed ringspun cotton",
        description_html: r#"<p>100% combed ringspun cotton</p>"#,
        image_url: "baby-cap-black",
        gallery_image_urls: &[
            "baby-cap-black",
            "baby-cap-gray",
            "baby-cap-white",
        ],
        price: 10.00,
        collections: &[
            "headwear",
            "kids",
        ],
    },

    ProductDef {
        handle: "acme-cowboy-hat",
        title: "Acme Cowboy Hat",
        description: r#"Part of our Buffalo collection, this cowboy hat is made in the USA of high-quality, weather-resistant 4X buffalo felt. Its classic Western profile features a classic cattleman crease, a 4" brim and a 4 1/2" regular oval crown. Additional details include a leather sweatband, satin lining, and a self-matching hat band with a three-piece silver-toned buckle set, as well as a hat box."#,
        description_html: r#"<p>Part of our Buffalo collection, this cowboy hat is made in the USA of high-quality, weather-resistant 4X buffalo felt. Its classic Western profile features a classic cattleman crease, a 4" brim and a 4 1/2" regular oval crown. Additional details include a leather sweatband, satin lining, and a self-matching hat band with a three-piece silver-toned buckle set, as well as a hat box.</p><ul>
<li>4" Brim</li>
<li>4 1/2" Regular Oval Crown</li>
<li>Cattleman Crease</li>
<li>Self-Matching Hat Band</li>
<li>3-Piece Silver Buckle Set</li>
<li>Hat Box</li>
<li>4X Wool Felt Construction</li>
<li>American Buffalo Collection</li>
<li>Made in the USA</li>
</ul>"#,
        image_url: "cowboy-hat-black-1",
        gallery_image_urls: &[
            "cowboy-hat-black-1",
            "cowboy-hat-black-2",
            "cowboy-hat-black-3",
            "cowboy-hat-black-4",
            "cowboy-hat-black-5",
        ],
        price: 160.00,
        collections: &[
            "headwear",
        ],
    },

    ProductDef {
        handle: "acme-cap",
        title: "Acme Cap",
        description: "100% peach-washed cotton.",
        description_html: r#"<p>100% peach-washed cotton.</p>"#,
        image_url: "hat-1",
        gallery_image_urls: &[
            "hat-1",
            "hat-2",
            "hat-3",
        ],
        price: 20.00,
        collections: &[
            "headwear",
        ],
    },

    ProductDef {
        handle: "acme-hoodie",
        title: "Acme Hoodie",
        description: "Fabric blend of Supima Cotton and Micromodal.",
        description_html: r#"<p>Fabric blend of Supima Cotton and Micromodal.</p>"#,
        image_url: "hoodie-1",
        gallery_image_urls: &[
            "hoodie-1",
            "hoodie-2",
        ],
        price: 50.00,
        collections: &[
            "hoodies",
        ],
    },

    ProductDef {
        handle: "acme-bomber-jacket",
        title: "Acme Bomber Jacket",
        description: "The multi-season must-have jacket: light and classic for daily wear, with a soft fleece lining for extra warmth.",
        description_html: r#"<p>The multi-season must-have jacket: light and classic for daily wear, with a soft fleece lining for extra warmth.</p>"#,
        image_url: "bomber-jacket-army",
        gallery_image_urls: &[
            "bomber-jacket-army",
            "bomber-jacket-black",
        ],
        price: 50.00,
        collections: &[
            "jackets",
        ],
    },

    ProductDef {
        handle: "acme-baby-onesie",
        title: "Acme Baby Onesie",
        description: "Short sleeve 5-oz, 100% combed ringspun cotton onesie",
        description_html: r#"<p>Short sleeve 5-oz, 100% combed ringspun cotton onesie</p>"#,
        image_url: "baby-onesie-beige-1",
        gallery_image_urls: &[
            "baby-onesie-beige-1",
            "baby-onesie-beige-2",
            "baby-onesie-black-1",
            "baby-onesie-black-2",
            "baby-onesie-white-1",
        ],
        price: 10.00,
        collections: &[
            "kids",
        ],
    },

    ProductDef {
        handle: "acme-pacifier",
        title: "Acme Pacifier",
        description: r#"This line of pacifiers has been thoughtfully designed for your baby's comfort. The pacifier allows your child to self-soothe in the most natural way possible."#,
        description_html: r#"<p>This line of pacifiers has been thoughtfully designed for your baby's comfort. The pacifier allows your child to self-soothe in the most natural way possible.</p>"#,
        image_url: "pacifier-1",
        gallery_image_urls: &[
            "pacifier-1",
            "pacifier-2",
        ],
        price: 10.00,
        collections: &[
            "kids",
        ],
    },

    ProductDef {
        handle: "acme-dog-sweater",
        title: "Acme Dog Sweater",
        description: r#"Keep your dog warm all winter long - When the cold weather hits, make sure your dog isn't shivering and stays warm with the soft and stretchy fleece dog sweater. Made with 90% polyester & 5% polyurethane to keep moisture out, freezing rain or snow, and to help keep warm air in, so your dog always stays warm. Our dog clothing is safe, durable, and made to last."#,
        description_html: r#"<p>Keep your dog warm all winter long - When the cold weather hits, make sure your dog isn't shivering and stays warm with the soft and stretchy fleece dog sweater. Made with 90% polyester &amp; 5% polyurethane to keep moisture out, freezing rain or snow, and to help keep warm air in, so your dog always stays warm. Our dog clothing is safe, durable, and made to last.</p>"#,
        image_url: "dog-sweater-1",
        gallery_image_urls: &[
            "dog-sweater-1",
            "dog-sweater-2",
        ],
        price: 20.00,
        collections: &[
            "pets",
        ],
    },

    ProductDef {
        handle: "acme-sticker",
        title: "Acme Sticker",
        description: "",
        description_html: "",
        image_url: "sticker",
        gallery_image_urls: &[
            "sticker",
        ],
        price: 4.00,
        collections: &[
            "stickers",
        ],
    },

    ProductDef {
        handle: "acme-rainbow-sticker",
        title: "Acme Rainbow Sticker",
        description: "",
        description_html: "",
        image_url: "sticker-rainbow",
        gallery_image_urls: &[
            "sticker-rainbow",
        ],
        price: 4.00,
        collections: &[
            "stickers",
        ],
    },
];

const HOME_FEATURED_HANDLES: &[&str] = &[
    "acme-geometric-circles-t-shirt",
    "acme-drawstring-bag",
    "acme-cup",
];

const HOME_CAROUSEL_HANDLES: &[&str] = &[
    "acme-mug",
    "acme-hoodie",
    "acme-baby-onesie",
    "acme-baby-cap",
    "acme-mug",
    "acme-hoodie",
    "acme-baby-onesie",
    "acme-baby-cap",
    "acme-mug",
    "acme-hoodie",
    "acme-baby-onesie",
    "acme-baby-cap",
];

const CATEGORY_DEFS: &[CategoryDef] = &[
    CategoryDef {
        handle: "bags",
        title: "Bags",
    },
    CategoryDef {
        handle: "drinkware",
        title: "Drinkware",
    },
    CategoryDef {
        handle: "electronics",
        title: "Electronics",
    },
    CategoryDef {
        handle: "footware",
        title: "Footware",
    },
    CategoryDef {
        handle: "headwear",
        title: "Headwear",
    },
    CategoryDef {
        handle: "hoodies",
        title: "Hoodies",
    },
    CategoryDef {
        handle: "jackets",
        title: "Jackets",
    },
    CategoryDef {
        handle: "kids",
        title: "Kids",
    },
    CategoryDef {
        handle: "pets",
        title: "Pets",
    },
    CategoryDef {
        handle: "shirts",
        title: "Shirts",
    },
    CategoryDef {
        handle: "stickers",
        title: "Stickers",
    },
];

const GRADIENTS: &[(&str, &str)] = &[
    ("#4f46e5", "#0ea5e9"),
    ("#f43f5e", "#f59e0b"),
    ("#22c55e", "#14b8a6"),
    ("#8b5cf6", "#ec4899"),
    ("#06b6d4", "#3b82f6"),
    ("#ef4444", "#f97316"),
    ("#84cc16", "#10b981"),
    ("#6366f1", "#8b5cf6"),
    ("#a855f7", "#ec4899"),
    ("#14b8a6", "#22c55e"),
    ("#f97316", "#eab308"),
    ("#0ea5e9", "#2563eb"),
];

const SHIRT_COLORS: &[&str] = &["Black", "Blue", "Gray", "Pink", "White"];
const BLACK_WHITE_COLORS: &[&str] = &["Black", "White"];
const BABY_CAP_COLORS: &[&str] = &["Black", "Gray", "White"];
const COWBOY_HAT_COLORS: &[&str] = &["Black", "Tan"];
const JACKET_COLORS: &[&str] = &["Army", "Black"];
const NO_OPTIONS: &[&str] = &[];

const APPAREL_SIZES: &[&str] = &["XS", "S", "M", "L", "XL", "XXL", "XXXL"];
const BAG_SIZES: &[&str] = &[
    "6 x 8 inch",
    "7 x 9 inch",
    "8 x 11 inch",
    "9 x 12 inch",
    "10 x 15 inch",
    "12 x 16 inch",
];
const FOOTWARE_SIZES: &[&str] = &[
    "1", "1.5", "2", "2.5", "3", "3.5", "4", "4.5", "5", "5.5", "6", "6.5", "7", "7.5", "8", "8.5",
    "9", "9.5", "10", "10.5", "11", "11.5", "12", "12.5", "13",
];
const DOG_SIZES: &[&str] = &[
    "0 - 5 lb",
    "5 - 20 lb",
    "20 - 50 lb",
    "50 - 75 lb",
    "75+ lb",
];
const BABY_ONESIE_SIZES: &[&str] = &["0-3M", "3-6M", "6-12M"];
const COWBOY_HAT_SIZES: &[&str] = &[
    "6 3/4", "6 7/8", "7", "7 1/8", "7 1/4", "7 3/8", "7 1/2", "7 5/8", "7 3/4",
];
const RELATED_HANDLES: &[&str] = &[
    "acme-cap",
    "acme-baby-cap",
    "acme-mug",
    "acme-hoodie",
    "acme-cup",
    "acme-t-shirt",
    "acme-rainbow-prism-t-shirt",
    "acme-baby-onesie",
    "acme-dog-sweater",
    "acme-geometric-circles-t-shirt",
    "acme-drawstring-bag",
    "acme-pacifier",
    "acme-mechanical-keyboard",
    "acme-slip-on-shoes",
    "acme-bomber-jacket",
    "acme-rainbow-sticker",
    "acme-sticker",
];

pub struct Catalog {
    products: Vec<Product>,
    categories: Vec<Category>,
    handle_index: HashMap<String, usize>,
    category_index: HashMap<String, Vec<usize>>,
    search_index: Vec<String>,
}

impl Catalog {
    pub fn generate() -> Self {
        let mut products = Vec::with_capacity(PRODUCT_DEFS.len());
        let mut handle_index = HashMap::with_capacity(PRODUCT_DEFS.len());
        let mut category_index = HashMap::with_capacity(CATEGORY_DEFS.len());
        let mut search_index = Vec::with_capacity(PRODUCT_DEFS.len());

        for (index, def) in PRODUCT_DEFS.iter().enumerate() {
            let handle = def.handle.to_string();
            let primary_collection = def.collections[0];
            let gradient_index = index % GRADIENTS.len();
            let alt_gradient_index = (index + 5) % GRADIENTS.len();
            let gradient = format!(
                "linear-gradient(135deg, {}, {})",
                GRADIENTS[gradient_index].0, GRADIENTS[gradient_index].1
            );
            let gradient_alt = format!(
                "linear-gradient(135deg, {}, {})",
                GRADIENTS[alt_gradient_index].0, GRADIENTS[alt_gradient_index].1
            );

            let colors = options_for(&handle, primary_collection, true)
                .iter()
                .map(|value| VariantOption {
                    value: (*value).to_string(),
                    available: true,
                })
                .collect();
            let sizes = options_for(&handle, primary_collection, false)
                .iter()
                .map(|value| VariantOption {
                    value: (*value).to_string(),
                    available: true,
                })
                .collect();

            let compare_at = if matches!(handle.as_str(), "acme-t-shirt" | "acme-bomber-jacket") {
                format!("${:.2}", def.price * 1.25)
            } else {
                String::new()
            };
            let image_alt_url = def
                .gallery_image_urls
                .get(1)
                .copied()
                .unwrap_or(def.image_url);
            let collections = def
                .collections
                .iter()
                .map(|value| (*value).to_string())
                .collect::<Vec<_>>();

            let mut tags = collections.clone();
            if index % 3 == 0 {
                tags.push("trending".to_string());
            }
            if index % 4 == 0 {
                tags.push("new-arrival".to_string());
            }

            let mut search_text = String::with_capacity(
                def.title.len()
                    + def.description.len()
                    + collections.iter().map(String::len).sum::<usize>()
                    + collections.len()
                    + 2,
            );
            search_text.push_str(&def.title.to_lowercase());
            if !def.description.is_empty() {
                search_text.push(' ');
                search_text.push_str(&def.description.to_lowercase());
            }
            for collection in &collections {
                search_text.push(' ');
                search_text.push_str(collection);
            }

            let product_index = products.len();
            handle_index.insert(handle.clone(), product_index);
            for collection in &collections {
                category_index
                    .entry(collection.clone())
                    .or_insert_with(|| Vec::with_capacity(4))
                    .push(product_index);
            }

            products.push(Product {
                handle,
                title: def.title.to_string(),
                description: def.description.to_string(),
                description_html: def.description_html.to_string(),
                price: format!("${:.2}", def.price),
                price_raw: def.price,
                compare_at,
                currency: "USD".to_string(),
                category: primary_collection.to_string(),
                gradient,
                gradient_alt,
                image_url: format!("_image/{}", def.image_url),
                image_alt_url: format!("_image/{}", image_alt_url),
                gallery_image_urls: def
                    .gallery_image_urls
                    .iter()
                    .map(|url| format!("_image/{}", url))
                    .collect(),
                available: true,
                tags,
                colors,
                sizes,
                collections,
            });
            search_index.push(search_text);
        }

        let categories = CATEGORY_DEFS
            .iter()
            .map(|category| Category {
                handle: category.handle.to_string(),
                title: category.title.to_string(),
                count: category_index.get(category.handle).map_or(0, Vec::len),
            })
            .collect();

        Self {
            products,
            categories,
            handle_index,
            category_index,
            search_index,
        }
    }

    pub fn product_count(&self) -> usize {
        self.products.len()
    }

    pub fn categories(&self) -> &[Category] {
        &self.categories
    }

    pub fn top_nav_categories(&self) -> Vec<&Category> {
        self.categories
            .iter()
            .filter(|category| matches!(category.handle.as_str(), "shirts" | "stickers"))
            .collect()
    }

    pub fn all(&self) -> Vec<&Product> {
        self.products.iter().collect()
    }

    pub fn by_handle(&self, handle: &str) -> Option<&Product> {
        self.handle_index
            .get(handle)
            .map(|index| &self.products[*index])
    }

    pub fn by_category(&self, category: &str) -> Vec<&Product> {
        self.category_index
            .get(category)
            .into_iter()
            .flat_map(|indices| indices.iter().map(|index| &self.products[*index]))
            .collect()
    }

    pub fn home_featured(&self) -> Vec<&Product> {
        self.collect_by_handles(HOME_FEATURED_HANDLES)
    }

    pub fn home_carousel(&self) -> Vec<&Product> {
        self.collect_by_handles(HOME_CAROUSEL_HANDLES)
    }

    pub fn related(&self, handle: &str, count: usize) -> Vec<&Product> {
        let mut related = Vec::with_capacity(count);

        for candidate in RELATED_HANDLES {
            if *candidate == handle {
                continue;
            }

            if let Some(product) = self.by_handle(candidate) {
                related.push(product);
            }

            if related.len() == count {
                return related;
            }
        }

        for product in &self.products {
            if product.handle == handle
                || related
                    .iter()
                    .any(|existing| existing.handle == product.handle)
            {
                continue;
            }

            related.push(product);
            if related.len() == count {
                break;
            }
        }

        related
    }

    pub fn search(&self, query: &str) -> Vec<&Product> {
        let needle = query.to_lowercase();
        self.search_index
            .iter()
            .zip(self.products.iter())
            .filter(|(search_text, _)| search_text.contains(&needle))
            .map(|(_, product)| product)
            .collect()
    }

    pub fn search_in_category(&self, category: &str, query: &str) -> Vec<&Product> {
        let needle = query.to_lowercase();
        self.category_index
            .get(category)
            .into_iter()
            .flat_map(|indices| {
                indices
                    .iter()
                    .filter(|index| self.search_index[**index].contains(&needle))
                    .map(|index| &self.products[*index])
            })
            .collect()
    }

    fn collect_by_handles<'a>(&'a self, handles: &[&str]) -> Vec<&'a Product> {
        handles
            .iter()
            .filter_map(|handle| {
                self.handle_index
                    .get(*handle)
                    .map(|index| &self.products[*index])
            })
            .collect()
    }
}

pub fn product_to_json(product: &Product) -> serde_json::Value {
    serde_json::json!({
        "handle": product.handle,
        "title": product.title,
        "price": product.price,
        "compareAt": product.compare_at,
        "gradient": product.gradient,
        "imageUrl": product.image_url,
        "category": product.category,
        "available": product.available,
    })
}

pub fn extend_product_detail_state(state: &mut Map<String, Value>, product: &Product) {
    let default_color = first_available_option(&product.colors);
    let default_size = first_available_option(&product.sizes);
    let mut option_groups = Vec::with_capacity(2);

    if !product.colors.is_empty() {
        option_groups.push(serde_json::json!({
            "name": "Color",
            "selected": "",
            "values": product.colors.iter().map(|color| serde_json::json!({
                "value": color.value,
                "unavailable": !color.available,
            })).collect::<Vec<_>>(),
        }));
    }

    if !product.sizes.is_empty() {
        option_groups.push(serde_json::json!({
            "name": "Size",
            "selected": "",
            "values": product.sizes.iter().map(|size| serde_json::json!({
                "value": size.value,
                "unavailable": !size.available,
            })).collect::<Vec<_>>(),
        }));
    }

    let images = gallery_images_json(product);

    state.insert("handle".into(), Value::String(product.handle.clone()));
    state.insert("title".into(), Value::String(product.title.clone()));
    state.insert("productTitle".into(), Value::String(product.title.clone()));
    state.insert(
        "description".into(),
        Value::String(product.description.clone()),
    );
    state.insert(
        "descriptionHtml".into(),
        Value::String(product.description_html.clone()),
    );
    state.insert("price".into(), Value::String(product.price.clone()));
    state.insert(
        "compareAt".into(),
        Value::String(product.compare_at.clone()),
    );
    state.insert(
        "hasCompareAt".into(),
        Value::Bool(!product.compare_at.is_empty()),
    );
    state.insert("currency".into(), Value::String(product.currency.clone()));
    state.insert("category".into(), Value::String(product.category.clone()));
    state.insert("gradient".into(), Value::String(product.gradient.clone()));
    state.insert(
        "gradientAlt".into(),
        Value::String(product.gradient_alt.clone()),
    );
    state.insert("imageUrl".into(), Value::String(product.image_url.clone()));
    state.insert(
        "imageAltUrl".into(),
        Value::String(product.image_alt_url.clone()),
    );
    state.insert("available".into(), Value::Bool(product.available));
    state.insert("tags".into(), serde_json::json!(product.tags));
    state.insert("optionGroups".into(), Value::Array(option_groups));
    state.insert("images".into(), Value::Array(images));
    state.insert("defaultColor".into(), Value::String(default_color.clone()));
    state.insert("defaultSize".into(), Value::String(default_size.clone()));
    state.insert("selectedColor".into(), Value::String(default_color));
    state.insert("selectedSize".into(), Value::String(default_size));
    state.insert("canSubmit".into(), Value::Bool(true));
}

pub fn products_to_json(products: &[&Product]) -> Vec<serde_json::Value> {
    products
        .iter()
        .map(|product| product_to_json(product))
        .collect()
}

pub fn categories_with_active_json(
    categories: &[Category],
    active: &str,
) -> Vec<serde_json::Value> {
    categories
        .iter()
        .map(|category| {
            serde_json::json!({
                "handle": category.handle,
                "title": category.title,
                "count": category.count,
                "active": category.handle == active,
            })
        })
        .collect()
}

pub fn sort_options_json(active: &str, base_path: &str, query: &str) -> Vec<serde_json::Value> {
    let options = [
        ("relevance", "Relevance"),
        ("trending-desc", "Trending"),
        ("latest-desc", "Latest arrivals"),
        ("price-asc", "Price: Low to high"),
        ("price-desc", "Price: High to low"),
    ];
    let encoded_query = encode_query_value(query);

    options
        .iter()
        .map(|(value, title)| {
            let href = if encoded_query.is_empty() {
                format!("{base_path}?sort={value}")
            } else {
                format!("{base_path}?q={encoded_query}&sort={value}")
            };
            serde_json::json!({
                "value": value,
                "title": title,
                "href": href,
                "active": *value == active,
            })
        })
        .collect()
}

pub fn sorted<'a>(mut products: Vec<&'a Product>, sort: &str) -> Vec<&'a Product> {
    match sort {
        "relevance" => products.sort_by_key(|product| relevance_rank(product.handle.as_str())),
        "price-asc" => products.sort_by(|left, right| {
            left.price_raw
                .partial_cmp(&right.price_raw)
                .unwrap_or(std::cmp::Ordering::Equal)
        }),
        "price-desc" => products.sort_by(|left, right| {
            right
                .price_raw
                .partial_cmp(&left.price_raw)
                .unwrap_or(std::cmp::Ordering::Equal)
        }),
        "latest-desc" => products.reverse(),
        _ => {}
    }
    products
}

fn relevance_rank(handle: &str) -> usize {
    match handle {
        "acme-cowboy-hat" => 0,
        "acme-geometric-circles-t-shirt" => 1,
        "acme-rainbow-prism-t-shirt" => 2,
        "acme-drawstring-bag" => 3,
        "acme-pacifier" => 4,
        "acme-mechanical-keyboard" => 5,
        "acme-t-shirt" => 6,
        "acme-hoodie" => 7,
        "acme-baby-onesie" => 8,
        "acme-sticker" => 9,
        "acme-mug" => 10,
        "acme-slip-on-shoes" => 11,
        "acme-rainbow-sticker" => 12,
        "acme-cap" => 13,
        "acme-dog-sweater" => 14,
        "acme-cup" => 15,
        "acme-bomber-jacket" => 16,
        "acme-baby-cap" => 17,
        _ => usize::MAX,
    }
}

fn options_for(handle: &str, category: &str, colors: bool) -> &'static [&'static str] {
    match (handle, colors) {
        (
            "acme-t-shirt" | "acme-rainbow-prism-t-shirt" | "acme-geometric-circles-t-shirt",
            true,
        ) => SHIRT_COLORS,
        (
            "acme-t-shirt" | "acme-rainbow-prism-t-shirt" | "acme-geometric-circles-t-shirt",
            false,
        ) => APPAREL_SIZES,
        ("acme-drawstring-bag", true) | ("acme-cup" | "acme-mug", true) => BLACK_WHITE_COLORS,
        ("acme-drawstring-bag", false) => BAG_SIZES,
        ("acme-slip-on-shoes", false) => FOOTWARE_SIZES,
        ("acme-cowboy-hat", true) => COWBOY_HAT_COLORS,
        ("acme-cowboy-hat", false) => COWBOY_HAT_SIZES,
        ("acme-baby-cap" | "acme-cap", true) => BABY_CAP_COLORS,
        ("acme-hoodie", false) => APPAREL_SIZES,
        ("acme-bomber-jacket", true) => JACKET_COLORS,
        ("acme-bomber-jacket", false) => APPAREL_SIZES,
        ("acme-baby-onesie", false) => BABY_ONESIE_SIZES,
        ("acme-dog-sweater", false) => DOG_SIZES,
        _ => {
            let _ = category;
            NO_OPTIONS
        }
    }
}

fn first_available_option(options: &[VariantOption]) -> String {
    options
        .iter()
        .find(|option| option.available)
        .or_else(|| options.first())
        .map_or_else(String::new, |option| option.value.clone())
}

fn encode_query_value(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            encoded.push(char::from(byte));
        } else {
            use std::fmt::Write;
            let _ = write!(encoded, "%{byte:02X}");
        }
    }
    encoded
}

fn gallery_images_json(product: &Product) -> Vec<serde_json::Value> {
    let gradients = [product.gradient.as_str(), product.gradient_alt.as_str()];

    product
        .gallery_image_urls
        .iter()
        .enumerate()
        .map(|(index, image_url)| {
            serde_json::json!({
                "index": index,
                "imageUrl": image_url,
                "gradient": gradients[index % gradients.len()],
                "active": index == 0,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{extend_product_detail_state, sort_options_json, Catalog};
    use serde_json::{Map, Value};

    fn product_detail_json(product: &super::Product) -> Value {
        let mut detail = Map::new();
        extend_product_detail_state(&mut detail, product);
        Value::Object(detail)
    }

    #[test]
    fn catalog_matches_live_collection_counts() {
        let catalog = Catalog::generate();

        assert_eq!(catalog.product_count(), 18);
        assert_eq!(catalog.by_category("bags").len(), 1);
        assert_eq!(catalog.by_category("drinkware").len(), 2);
        assert_eq!(catalog.by_category("electronics").len(), 1);
        assert_eq!(catalog.by_category("footware").len(), 1);
        assert_eq!(catalog.by_category("headwear").len(), 3);
        assert_eq!(catalog.by_category("hoodies").len(), 1);
        assert_eq!(catalog.by_category("jackets").len(), 1);
        assert_eq!(catalog.by_category("kids").len(), 3);
        assert_eq!(catalog.by_category("pets").len(), 1);
        assert_eq!(catalog.by_category("shirts").len(), 3);
        assert_eq!(catalog.by_category("stickers").len(), 2);
    }

    #[test]
    fn product_can_belong_to_multiple_collections() {
        let catalog = Catalog::generate();
        let product = catalog
            .by_handle("acme-baby-cap")
            .expect("baby cap should exist");

        assert!(product.belongs_to("headwear"));
        assert!(product.belongs_to("kids"));
    }

    #[test]
    fn products_without_variants_emit_empty_option_groups() {
        let catalog = Catalog::generate();
        let keyboard = catalog
            .by_handle("acme-mechanical-keyboard")
            .expect("keyboard should exist");
        let detail = product_detail_json(keyboard);

        assert_eq!(
            detail["optionGroups"]
                .as_array()
                .map_or(usize::MAX, Vec::len),
            0
        );
        assert_eq!(detail["defaultColor"], "");
        assert_eq!(detail["defaultSize"], "");
    }

    #[test]
    fn sort_options_match_live_param_names() {
        let sort_options = sort_options_json("trending-desc", "/search/shirts", "");
        let hrefs = sort_options
            .iter()
            .map(|item| item["href"].as_str().unwrap_or_default())
            .collect::<Vec<_>>();

        assert!(hrefs.contains(&"/search/shirts?sort=trending-desc"));
        assert!(hrefs.contains(&"/search/shirts?sort=latest-desc"));
    }

    #[test]
    fn home_featured_matches_live_order() {
        let catalog = Catalog::generate();
        let handles = catalog
            .home_featured()
            .iter()
            .map(|product| product.handle.as_str())
            .collect::<Vec<_>>();

        assert_eq!(
            handles,
            vec![
                "acme-geometric-circles-t-shirt",
                "acme-drawstring-bag",
                "acme-cup",
            ]
        );
    }

    #[test]
    fn home_carousel_matches_live_order() {
        let catalog = Catalog::generate();
        let handles = catalog
            .home_carousel()
            .iter()
            .map(|product| product.handle.as_str())
            .collect::<Vec<_>>();

        assert_eq!(
            handles,
            vec![
                "acme-mug",
                "acme-hoodie",
                "acme-baby-onesie",
                "acme-baby-cap",
                "acme-mug",
                "acme-hoodie",
                "acme-baby-onesie",
                "acme-baby-cap",
                "acme-mug",
                "acme-hoodie",
                "acme-baby-onesie",
                "acme-baby-cap",
            ]
        );
    }

    #[test]
    fn scraped_product_data_is_embedded_in_catalog() {
        let catalog = Catalog::generate();
        let cowboy_hat = catalog
            .by_handle("acme-cowboy-hat")
            .expect("cowboy hat should exist");
        let drawstring_bag = catalog
            .by_handle("acme-drawstring-bag")
            .expect("drawstring bag should exist");

        assert_eq!(cowboy_hat.gallery_image_urls.len(), 5);
        assert!(cowboy_hat
            .description_html
            .contains("American Buffalo Collection"));
        assert!(drawstring_bag
            .description
            .contains("Strong 210D ripstop nylon drawstring bag"));
        assert!(drawstring_bag
            .description_html
            .contains("Available in multiple sizes"));
    }

    #[test]
    fn search_is_case_insensitive_after_indexing() {
        let catalog = Catalog::generate();

        let shirts = catalog.search("PRISM");
        let stickers = catalog.search("stickers");
        let category_matches = catalog.search_in_category("shirts", "circles");

        assert_eq!(
            shirts.first().map(|product| product.handle.as_str()),
            Some("acme-rainbow-prism-t-shirt")
        );
        assert_eq!(stickers.len(), 2);
        assert_eq!(category_matches.len(), 1);
        assert_eq!(
            category_matches[0].handle.as_str(),
            "acme-geometric-circles-t-shirt"
        );
    }
}
