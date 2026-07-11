// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.
use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use std::hint::black_box;
use webui_parser::{
    plugin::fast_v2::FastV2ParserPlugin, scan_hydration_attributes, CssStrategy, HtmlParser,
};

fn build_hydration_script(properties: usize, noise_blocks: usize) -> String {
    let mut source = String::with_capacity(properties * 48 + noise_blocks * 256 + 128);
    source.push_str("export class BenchElement {\n");
    for idx in 0..properties {
        source.push_str("  @observable value");
        source.push_str(&idx.to_string());
        source.push_str(" = 0;\n");
    }
    for idx in 0..noise_blocks {
        source.push_str("  // @observable lineComment");
        source.push_str(&idx.to_string());
        source.push_str(" = 1;\n");
        source.push_str("  /* @attr blockComment");
        source.push_str(&idx.to_string());
        source.push_str(" = 'x'; */\n");
        source.push_str("  text");
        source.push_str(&idx.to_string());
        source.push_str(" = \"@observable stringValue");
        source.push_str(&idx.to_string());
        source.push_str(" = 1\";\n");
        source.push_str("  template");
        source.push_str(&idx.to_string());
        source.push_str(" = `@attr templateValue");
        source.push_str(&idx.to_string());
        source.push_str(" = 'x'`;\n");
        source.push_str("  regex");
        source.push_str(&idx.to_string());
        source.push_str(" = /@observable regexValue");
        source.push_str(&idx.to_string());
        source.push_str("/;\n");
    }
    source.push_str("}\n");
    source
}

fn build_simple_template() -> String {
    let mut html = String::with_capacity(256);
    html.push_str("<body>");
    html.push_str("<h1>{{title}}</h1>");
    html.push_str("<p>{{description}}</p>");
    html.push_str("</body>");
    html
}

fn build_attribute_heavy_template(repetitions: usize) -> String {
    let mut html = String::with_capacity(repetitions * 220 + 64);
    html.push_str("<body><div class=\"root\">");

    for idx in 0..repetitions {
        html.push_str("<x-bench-button ");
        html.push_str("class=\"btn {{theme}} {{size}}\" ");
        html.push_str("?disabled=\"{{isDisabled}}\" ");
        html.push_str(":config=\"{{settings}}\" ");
        html.push_str("title=\"item ");
        html.push_str(&idx.to_string());
        html.push_str(" {{tooltip}}\" ");
        html.push_str("data-x=\"{{meta}}\">");
        html.push_str("{{label}}</x-bench-button>");
    }

    html.push_str("</div></body>");
    html
}

fn build_directive_heavy_template(loop_depth: usize, leaves: usize) -> String {
    let mut html = String::with_capacity(loop_depth * 80 + leaves * 120 + 128);
    html.push_str("<body>");

    for depth in 0..loop_depth {
        html.push_str("<for each=\"item");
        html.push_str(&depth.to_string());
        html.push_str(" in list");
        html.push_str(&depth.to_string());
        html.push_str("\">");
    }

    for leaf in 0..leaves {
        html.push_str("<if condition=\"item");
        html.push_str(&(leaf % loop_depth.max(1)).to_string());
        html.push_str(".enabled && globalReady\">");
        html.push_str("<div class=\"row\">{{item");
        html.push_str(&(leaf % loop_depth.max(1)).to_string());
        html.push_str(".name}}</div>");
        html.push_str("</if>");
    }

    for _ in 0..loop_depth {
        html.push_str("</for>");
    }

    html.push_str("</body>");
    html
}

fn build_component_heavy_template(components: usize) -> String {
    let component_names = ["x-card", "x-panel", "x-banner", "x-dialog", "x-item"];
    let mut html = String::with_capacity(components * 140 + 64);
    html.push_str("<body>");

    for idx in 0..components {
        let component_name = component_names[idx % component_names.len()];
        html.push('<');
        html.push_str(component_name);
        html.push_str(" title=\"{{title}}\" props=\"{{props}}\" ?active=\"{{active}}\">");
        html.push_str("<span>slot ");
        html.push_str(&idx.to_string());
        html.push_str("</span></");
        html.push_str(component_name);
        html.push('>');
    }

    html.push_str("</body>");
    html
}

fn build_style_heavy_template(blocks: usize) -> String {
    let mut html = String::with_capacity(blocks * 220 + 128);
    html.push_str("<body>");

    for idx in 0..blocks {
        html.push_str("<style>");
        html.push_str(".c");
        html.push_str(&idx.to_string());
        html.push_str(" { color: var(--webui-color);");
        html.push_str(" background: linear-gradient(#111, #222);");
        html.push_str(" padding: 8px; margin: 4px; }");
        html.push_str("</style>");
    }

    html.push_str("<div>{{content}}</div></body>");
    html
}

fn build_todo_app_template() -> String {
    let mut html = String::with_capacity(900);
    html.push_str("<!DOCTYPE html>\n");
    html.push_str("<html lang=\"{{language}}\" dir=\"{{textdirection}}\">\n");
    html.push_str("<head><meta charset=\"UTF-8\"><title>{{title}}</title>\n");
    html.push_str("<style>:host { display: block; font-family: sans-serif; } ");
    html.push_str(".container { max-width: 600px; margin: 0 auto; padding: 16px; } ");
    html.push_str(
        ".header { display: flex; justify-content: space-between; align-items: center; } ",
    );
    html.push_str(".count { color: var(--webui-muted); font-size: 14px; }</style>\n");
    html.push_str("</head>\n<body>\n");
    html.push_str("<div class=\"container\">\n");
    html.push_str("  <div class=\"header\">\n");
    html.push_str("    <h1>{{title}}</h1>\n");
    html.push_str("    <span class=\"count\">{{remainingCount}} remaining</span>\n");
    html.push_str("  </div>\n");
    html.push_str("  <div class=\"input-row\">\n");
    html.push_str("    <input type=\"text\" placeholder=\"Add a new item...\" />\n");
    html.push_str("    <button class=\"btn btn-primary\">Add</button>\n");
    html.push_str("  </div>\n");
    html.push_str("  <ul class=\"todo-list\">\n");
    html.push_str("    <for each=\"item in items\">\n");
    html.push_str("      <li class=\"todo-item\" data-id=\"{{item.id}}\">\n");
    html.push_str("        <if condition=\"item.state == 'done'\">\n");
    html.push_str("          <span class=\"done\">{{item.title}}</span>\n");
    html.push_str("        </if>\n");
    html.push_str("        <if condition=\"item.state != 'done'\">\n");
    html.push_str("          <span class=\"pending\">{{item.title}}</span>\n");
    html.push_str("        </if>\n");
    html.push_str(
        "        <button class=\"btn-small\" title=\"Toggle {{item.title}}\">✓</button>\n",
    );
    html.push_str("        <button class=\"btn-small btn-danger\" title=\"Delete\">✕</button>\n");
    html.push_str("      </li>\n");
    html.push_str("    </for>\n");
    html.push_str("  </ul>\n");
    html.push_str("  <if condition=\"showFooter\">\n");
    html.push_str("    <footer class=\"footer\">\n");
    html.push_str("      <p>{{footerText}}</p>\n");
    html.push_str("      <a href=\"{{helpUrl}}\">Help</a>\n");
    html.push_str("    </footer>\n");
    html.push_str("  </if>\n");
    html.push_str("</div>\n");
    html.push_str("</body>\n</html>");
    html
}

fn build_component_template() -> String {
    let mut html = String::with_capacity(500);
    html.push_str("<template shadowrootmode=\"open\">\n");
    html.push_str("<style>:host { display: block; } ");
    html.push_str(
        ".card { border: 1px solid var(--webui-border); border-radius: 8px; padding: 16px; } ",
    );
    html.push_str(".card-header { font-weight: bold; margin-bottom: 8px; } ");
    html.push_str(".card-body { color: var(--webui-text); } ");
    html.push_str(
        ".card-footer { margin-top: 12px; font-size: 12px; color: var(--webui-muted); }</style>\n",
    );
    html.push_str("<div class=\"card {{variant}}\">\n");
    html.push_str("  <div class=\"card-header\">{{title}}</div>\n");
    html.push_str("  <div class=\"card-body\">\n");
    html.push_str("    <slot></slot>\n");
    html.push_str("  </div>\n");
    html.push_str("  <if condition=\"hasFooter\">\n");
    html.push_str("    <div class=\"card-footer\">\n");
    html.push_str("      <span>{{footerText}}</span>\n");
    html.push_str("      <a href=\"{{footerLink}}\" ?hidden=\"{{hideLink}}\">Details</a>\n");
    html.push_str("    </div>\n");
    html.push_str("  </if>\n");
    html.push_str("</div>\n");
    html.push_str("</template>");
    html
}

fn build_text_heavy_template() -> String {
    let mut html = String::with_capacity(1100);
    html.push_str("<article class=\"post\">\n");
    html.push_str("  <header>\n");
    html.push_str("    <h1>{{post.title}}</h1>\n");
    html.push_str("    <p class=\"meta\">By {{post.author}} on {{post.date}}</p>\n");
    html.push_str("  </header>\n");
    html.push_str("  <section class=\"body\">\n");
    html.push_str("    <p>Lorem ipsum dolor sit amet, consectetur adipiscing elit. ");
    html.push_str("Sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. ");
    html.push_str("Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris.</p>\n");
    html.push_str("    <p>Duis aute irure dolor in reprehenderit in voluptate velit esse ");
    html.push_str("cillum dolore eu fugiat nulla pariatur. ");
    html.push_str("Excepteur sint occaecat cupidatat non proident.</p>\n");
    html.push_str("    <blockquote>{{post.pullQuote}}</blockquote>\n");
    html.push_str("    <p>Sed ut perspiciatis unde omnis iste natus error sit voluptatem ");
    html.push_str("accusantium doloremque laudantium, totam rem aperiam, eaque ipsa quae ");
    html.push_str(
        "ab illo inventore veritatis et quasi architecto beatae vitae dicta sunt explicabo.</p>\n",
    );
    html.push_str(
        "    <p>Nemo enim ipsam voluptatem quia voluptas sit aspernatur aut odit aut fugit, ",
    );
    html.push_str(
        "sed quia consequuntur magni dolores eos qui ratione voluptatem sequi nesciunt.</p>\n",
    );
    html.push_str("  </section>\n");
    html.push_str("  <footer>\n");
    html.push_str("    <p>Tags: {{post.tags}}</p>\n");
    html.push_str("    <p>Category: {{post.category}}</p>\n");
    html.push_str("  </footer>\n");
    html.push_str("</article>");
    html
}

fn build_dashboard_template() -> String {
    let mut html = String::with_capacity(1300);
    html.push_str("<div class=\"dashboard\">\n");
    html.push_str("  <nav class=\"sidebar\">\n");
    html.push_str("    <for each=\"link in navLinks\">\n");
    html.push_str("      <a href=\"{{link.url}}\" class=\"nav-link {{link.activeClass}}\" ");
    html.push_str("?disabled=\"{{link.disabled}}\">{{link.label}}</a>\n");
    html.push_str("    </for>\n");
    html.push_str("  </nav>\n");
    html.push_str("  <main class=\"content\">\n");
    html.push_str("    <if condition=\"isAdmin\">\n");
    html.push_str("      <div class=\"admin-bar\">\n");
    html.push_str("        <span>Admin: {{user.name}}</span>\n");
    html.push_str("        <button class=\"btn\">Settings</button>\n");
    html.push_str("      </div>\n");
    html.push_str("    </if>\n");
    html.push_str("    <h2>{{pageTitle}}</h2>\n");
    html.push_str("    <x-stats-card title=\"{{stats.usersTitle}}\" :data=\"{{stats.users}}\" ");
    html.push_str("?loading=\"{{stats.loading}}\">\n");
    html.push_str("      <span>{{stats.usersCount}} total</span>\n");
    html.push_str("    </x-stats-card>\n");
    html.push_str("    <x-stats-card title=\"{{stats.ordersTitle}}\" :data=\"{{stats.orders}}\" ");
    html.push_str("?loading=\"{{stats.loading}}\">\n");
    html.push_str("      <span>{{stats.ordersCount}} total</span>\n");
    html.push_str("    </x-stats-card>\n");
    html.push_str("    <div class=\"table-container\">\n");
    html.push_str("      <for each=\"row in tableData\">\n");
    html.push_str("        <div class=\"table-row\" data-id=\"{{row.id}}\">\n");
    html.push_str("          <span class=\"cell\">{{row.name}}</span>\n");
    html.push_str("          <span class=\"cell\">{{row.email}}</span>\n");
    html.push_str("          <span class=\"cell\">{{row.role}}</span>\n");
    html.push_str("          <if condition=\"row.isActive\">\n");
    html.push_str("            <span class=\"badge active\">Active</span>\n");
    html.push_str("          </if>\n");
    html.push_str("          <if condition=\"!row.isActive\">\n");
    html.push_str("            <span class=\"badge inactive\">Inactive</span>\n");
    html.push_str("          </if>\n");
    html.push_str("        </div>\n");
    html.push_str("      </for>\n");
    html.push_str("    </div>\n");
    html.push_str("  </main>\n");
    html.push_str("</div>");
    html
}

fn build_deeply_nested_template(depth: usize) -> String {
    let mut html = String::with_capacity(depth * 11 + 16);
    for _ in 0..depth {
        html.push_str("<div>");
    }
    html.push_str("leaf");
    for _ in 0..depth {
        html.push_str("</div>");
    }
    html
}

fn build_many_siblings_template(count: usize) -> String {
    let mut html = String::with_capacity(count * 32 + 16);
    html.push_str("<section>");
    for idx in 0..count {
        html.push_str("<span data-i=\"");
        html.push_str(&idx.to_string());
        html.push_str("\">item</span>");
    }
    html.push_str("</section>");
    html
}

fn build_large_text_template(bytes: usize) -> String {
    let mut html = String::with_capacity(bytes + 32);
    html.push_str("<article>");
    while html.len() < bytes {
        html.push_str("Lorem ipsum dolor sit amet. ");
    }
    html.push_str("</article>");
    html
}

fn build_large_style_template(rules: usize) -> String {
    let mut html = String::with_capacity(rules * 72 + 32);
    html.push_str("<style>");
    for idx in 0..rules {
        html.push_str(".c");
        html.push_str(&idx.to_string());
        html.push_str("{color:var(--color);padding:4px;margin:2px}");
    }
    html.push_str("</style>");
    html
}

fn build_nested_article_template(depth: usize) -> String {
    let mut html = String::with_capacity(depth * 11 + 16);
    for _ in 0..depth {
        html.push_str("<div>");
    }
    html.push_str("nested");
    for _ in 0..depth {
        html.push_str("</div>");
    }
    html
}

fn parser_with_bench_components() -> HtmlParser {
    let mut parser = HtmlParser::new();
    register_bench_components(&mut parser);
    parser
}

fn parser_with_bench_components_and_options(
    options: impl Into<webui_parser::ParserOptions>,
) -> HtmlParser {
    let mut parser = HtmlParser::with_options(options);
    register_bench_components(&mut parser);
    parser
}

fn parser_with_bench_components_and_fast_plugin() -> HtmlParser {
    let mut parser = HtmlParser::with_plugin(Box::new(FastV2ParserPlugin::new()));
    register_bench_components(&mut parser);
    parser
}

fn register_bench_components(parser: &mut HtmlParser) {
    let registry = parser.component_registry_mut();
    registry
        .register_component(webui_parser::ComponentRegistration::new(
            "x-bench-button",
            "<slot></slot>",
            None,
            true,
        ))
        .unwrap_or_else(|error| panic!("failed to register x-bench-button: {error}"));
    registry
        .register_component(webui_parser::ComponentRegistration::new(
            "x-card",
            "<slot></slot>",
            None,
            true,
        ))
        .unwrap_or_else(|error| panic!("failed to register x-card: {error}"));
    registry
        .register_component(webui_parser::ComponentRegistration::new(
            "x-panel",
            "<slot></slot>",
            None,
            true,
        ))
        .unwrap_or_else(|error| panic!("failed to register x-panel: {error}"));
    registry
        .register_component(webui_parser::ComponentRegistration::new(
            "x-banner",
            "<slot></slot>",
            None,
            true,
        ))
        .unwrap_or_else(|error| panic!("failed to register x-banner: {error}"));
    registry
        .register_component(webui_parser::ComponentRegistration::new(
            "x-dialog",
            "<slot></slot>",
            None,
            true,
        ))
        .unwrap_or_else(|error| panic!("failed to register x-dialog: {error}"));
    registry
        .register_component(webui_parser::ComponentRegistration::new(
            "x-item",
            "<slot></slot>",
            None,
            true,
        ))
        .unwrap_or_else(|error| panic!("failed to register x-item: {error}"));
    registry
        .register_component(webui_parser::ComponentRegistration::new(
            "x-stats-card",
            "<slot></slot>",
            None,
            true,
        ))
        .unwrap_or_else(|error| panic!("failed to register x-stats-card: {error}"));
}

fn parser_parse_reuse_bench(c: &mut Criterion) {
    let mut group = c.benchmark_group("parser_parse_reuse");

    let scenarios = [
        ("simple", build_simple_template()),
        ("attributes_100", build_attribute_heavy_template(100)),
        ("directives_l4_n16", build_directive_heavy_template(4, 16)),
        ("styles_40", build_style_heavy_template(40)),
    ];

    for (name, input) in scenarios {
        group.throughput(Throughput::Bytes(input.len() as u64));
        group.bench_with_input(BenchmarkId::new("reuse", name), &input, |b, html| {
            let mut parser = parser_with_bench_components();
            b.iter(|| {
                parser
                    .parse("index.html", black_box(html))
                    .unwrap_or_else(|error| panic!("parse failed for scenario {name}: {error}"));
            });
        });
    }

    group.finish();
}

fn parser_parse_fresh_vs_reuse(c: &mut Criterion) {
    let mut group = c.benchmark_group("parser_fresh_vs_reuse");
    let input = build_attribute_heavy_template(150);
    group.throughput(Throughput::Bytes(input.len() as u64));

    group.bench_function("fresh_per_iteration", |b| {
        b.iter(|| {
            let mut parser = parser_with_bench_components();
            parser
                .parse("index.html", black_box(&input))
                .unwrap_or_else(|error| panic!("fresh parse failed: {error}"));
        });
    });

    group.bench_function("reuse_single_parser", |b| {
        let mut parser = parser_with_bench_components();
        b.iter(|| {
            parser
                .parse("index.html", black_box(&input))
                .unwrap_or_else(|error| panic!("reuse parse failed: {error}"));
        });
    });

    group.finish();
}

fn parser_plugin_bench(c: &mut Criterion) {
    let mut group = c.benchmark_group("parser_plugin_fast");
    let input = build_attribute_heavy_template(120);
    group.throughput(Throughput::Bytes(input.len() as u64));

    group.bench_function("without_plugin", |b| {
        let mut parser = parser_with_bench_components();
        b.iter(|| {
            parser
                .parse("index.html", black_box(&input))
                .unwrap_or_else(|error| panic!("parse without plugin failed: {error}"));
        });
    });

    group.bench_function("with_fast_plugin", |b| {
        let mut parser = parser_with_bench_components_and_fast_plugin();
        b.iter(|| {
            parser
                .parse("index.html", black_box(&input))
                .unwrap_or_else(|error| panic!("parse with fast plugin failed: {error}"));
        });
    });

    group.finish();
}

fn parser_css_strategy_bench(c: &mut Criterion) {
    let mut group = c.benchmark_group("parser_css_strategy");
    let input = build_component_heavy_template(120);
    group.throughput(Throughput::Bytes(input.len() as u64));

    group.bench_function("external_css", |b| {
        let mut parser = parser_with_bench_components_and_options(CssStrategy::Link);
        b.iter(|| {
            parser
                .parse("index.html", black_box(&input))
                .unwrap_or_else(|error| panic!("external css parse failed: {error}"));
        });
    });

    group.bench_function("inline_css", |b| {
        let mut parser = parser_with_bench_components_and_options(CssStrategy::Style);
        b.iter(|| {
            parser
                .parse("index.html", black_box(&input))
                .unwrap_or_else(|error| panic!("inline css parse failed: {error}"));
        });
    });

    group.finish();
}

fn parser_size_sweep_bench(c: &mut Criterion) {
    let mut group = c.benchmark_group("parser_size_sweep");

    for &size in &[10usize, 50, 100, 200, 400] {
        let input = build_attribute_heavy_template(size);
        group.throughput(Throughput::Bytes(input.len() as u64));
        group.bench_with_input(BenchmarkId::new("attrs", size), &input, |b, html| {
            let mut parser = parser_with_bench_components();
            b.iter(|| {
                parser
                    .parse("index.html", black_box(html))
                    .unwrap_or_else(|error| panic!("size sweep parse failed for {size}: {error}"));
            });
        });
    }

    group.finish();
}

fn parser_realistic_bench(c: &mut Criterion) {
    let mut group = c.benchmark_group("parser_realistic");

    let scenarios = [
        ("todo_app", build_todo_app_template()),
        ("component", build_component_template()),
        ("text_heavy", build_text_heavy_template()),
        ("dashboard", build_dashboard_template()),
    ];

    for (name, input) in scenarios {
        group.throughput(Throughput::Bytes(input.len() as u64));
        group.bench_with_input(BenchmarkId::new("parse", name), &input, |b, html| {
            let mut parser = parser_with_bench_components();
            b.iter(|| {
                parser
                    .parse("index.html", black_box(html))
                    .unwrap_or_else(|error| panic!("realistic parse failed for {name}: {error}"));
            });
        });
    }

    group.finish();
}

fn parser_text_vs_directive_bench(c: &mut Criterion) {
    let mut group = c.benchmark_group("parser_text_vs_directive");

    let text_input = build_text_heavy_template();
    let directive_input = build_directive_heavy_template(3, 12);

    // Normalize comparison by using similar-sized inputs
    group.throughput(Throughput::Bytes(text_input.len() as u64));
    group.bench_function("text_heavy", |b| {
        let mut parser = parser_with_bench_components();
        b.iter(|| {
            parser
                .parse("index.html", black_box(&text_input))
                .unwrap_or_else(|error| panic!("text heavy parse failed: {error}"));
        });
    });

    group.throughput(Throughput::Bytes(directive_input.len() as u64));
    group.bench_function("directive_heavy", |b| {
        let mut parser = parser_with_bench_components();
        b.iter(|| {
            parser
                .parse("index.html", black_box(&directive_input))
                .unwrap_or_else(|error| panic!("directive heavy parse failed: {error}"));
        });
    });

    group.finish();
}

fn parser_adversarial_bench(c: &mut Criterion) {
    let mut group = c.benchmark_group("parser_adversarial");

    let scenarios = [
        ("deep_nesting_128", build_deeply_nested_template(128)),
        ("many_siblings_1000", build_many_siblings_template(1000)),
        ("large_text_64k", build_large_text_template(64 * 1024)),
        ("large_style_1000", build_large_style_template(1000)),
        ("nested_closed_128", build_nested_article_template(128)),
    ];

    for (name, input) in scenarios {
        group.throughput(Throughput::Bytes(input.len() as u64));
        group.bench_with_input(BenchmarkId::new("parse", name), &input, |b, html| {
            let mut parser = parser_with_bench_components();
            b.iter(|| {
                parser
                    .parse("index.html", black_box(html))
                    .unwrap_or_else(|error| panic!("adversarial parse failed for {name}: {error}"));
            });
        });
    }

    group.finish();
}

fn hydration_scanner_bench(c: &mut Criterion) {
    let mut group = c.benchmark_group("hydration_scanner");
    let scenarios = [
        ("reactive_256", build_hydration_script(256, 0)),
        ("mixed_noise_256", build_hydration_script(16, 256)),
    ];

    for (name, source) in scenarios {
        group.throughput(Throughput::Bytes(source.len() as u64));
        group.bench_with_input(BenchmarkId::new("scan", name), &source, |b, input| {
            b.iter(|| {
                let names = scan_hydration_attributes(black_box(input));
                black_box(names.len());
            });
        });
    }

    group.finish();
}

criterion_group!(
    benches,
    parser_parse_reuse_bench,
    parser_parse_fresh_vs_reuse,
    parser_plugin_bench,
    parser_css_strategy_bench,
    parser_size_sweep_bench,
    parser_realistic_bench,
    parser_text_vs_directive_bench,
    parser_adversarial_bench,
    hydration_scanner_bench
);
criterion_main!(benches);
