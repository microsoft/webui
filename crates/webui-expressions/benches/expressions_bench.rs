// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.
use criterion::{criterion_group, criterion_main, Criterion, Throughput};
use serde_json::{json, Map, Value};
use std::hint::black_box;
use webui_expressions::evaluate;
use webui_protocol::{ComparisonOperator, ConditionExpr, LogicalOperator};

fn expr_identifier_bench(c: &mut Criterion) {
    let mut group = c.benchmark_group("expr_identifier");

    let state = json!({
        "isAdmin": true,
        "isActive": false,
        "count": 42,
        "name": "Alice",
        "user": {
            "profile": {
                "verified": true
            }
        }
    });

    // Simple boolean identifier — fastest path
    let cond_bool = ConditionExpr::identifier("isAdmin");
    group.bench_function("boolean", |b| {
        b.iter(|| evaluate(black_box(&cond_bool), black_box(&state)));
    });

    // Number truthiness
    let cond_num = ConditionExpr::identifier("count");
    group.bench_function("number", |b| {
        b.iter(|| evaluate(black_box(&cond_num), black_box(&state)));
    });

    // String truthiness
    let cond_str = ConditionExpr::identifier("name");
    group.bench_function("string", |b| {
        b.iter(|| evaluate(black_box(&cond_str), black_box(&state)));
    });

    // Deep path identifier
    let cond_deep = ConditionExpr::identifier("user.profile.verified");
    group.bench_function("deep_path", |b| {
        b.iter(|| evaluate(black_box(&cond_deep), black_box(&state)));
    });

    group.finish();
}

fn expr_predicate_bench(c: &mut Criterion) {
    let mut group = c.benchmark_group("expr_predicate");

    let state = json!({
        "status": "active",
        "age": 25,
        "role": "admin",
        "score": 95.5
    });

    // String equality
    let eq_str = ConditionExpr::predicate("status", ComparisonOperator::Equal, "'active'");
    group.bench_function("string_eq", |b| {
        b.iter(|| evaluate(black_box(&eq_str), black_box(&state)));
    });

    // String inequality
    let neq_str = ConditionExpr::predicate("status", ComparisonOperator::NotEqual, "'inactive'");
    group.bench_function("string_neq", |b| {
        b.iter(|| evaluate(black_box(&neq_str), black_box(&state)));
    });

    // Numeric greater-than (requires type coercion)
    let gt_num = ConditionExpr::predicate("age", ComparisonOperator::GreaterThan, "18");
    group.bench_function("numeric_gt", |b| {
        b.iter(|| evaluate(black_box(&gt_num), black_box(&state)));
    });

    // Numeric less-than-or-equal
    let lte_num = ConditionExpr::predicate("score", ComparisonOperator::LessThanOrEqual, "100");
    group.bench_function("numeric_lte", |b| {
        b.iter(|| evaluate(black_box(&lte_num), black_box(&state)));
    });

    // Variable vs variable comparison
    let var_cmp = ConditionExpr::predicate("age", ComparisonOperator::LessThan, "score");
    group.bench_function("var_vs_var", |b| {
        b.iter(|| evaluate(black_box(&var_cmp), black_box(&state)));
    });

    group.finish();
}

fn expr_compound_bench(c: &mut Criterion) {
    let mut group = c.benchmark_group("expr_compound");

    let state = json!({
        "isAdmin": true,
        "isActive": true,
        "hasPermission": true,
        "isOwner": false,
        "isEditor": false,
        "isModerator": false,
    });

    // Two-term AND
    let and_2 = ConditionExpr::compound(
        ConditionExpr::identifier("isAdmin"),
        LogicalOperator::And,
        ConditionExpr::identifier("isActive"),
    );
    group.bench_function("and_2_terms", |b| {
        b.iter(|| evaluate(black_box(&and_2), black_box(&state)));
    });

    // Three-term AND (chained)
    let and_3 = ConditionExpr::compound(
        ConditionExpr::compound(
            ConditionExpr::identifier("isAdmin"),
            LogicalOperator::And,
            ConditionExpr::identifier("isActive"),
        ),
        LogicalOperator::And,
        ConditionExpr::identifier("hasPermission"),
    );
    group.bench_function("and_3_terms", |b| {
        b.iter(|| evaluate(black_box(&and_3), black_box(&state)));
    });

    // Two-term OR (short-circuit on first true)
    let or_short = ConditionExpr::compound(
        ConditionExpr::identifier("isAdmin"),
        LogicalOperator::Or,
        ConditionExpr::identifier("isOwner"),
    );
    group.bench_function("or_short_circuit", |b| {
        b.iter(|| evaluate(black_box(&or_short), black_box(&state)));
    });

    // Two-term OR (must evaluate both)
    let or_full = ConditionExpr::compound(
        ConditionExpr::identifier("isOwner"),
        LogicalOperator::Or,
        ConditionExpr::identifier("isAdmin"),
    );
    group.bench_function("or_full_eval", |b| {
        b.iter(|| evaluate(black_box(&or_full), black_box(&state)));
    });

    // AND short-circuit (first false, skip rest)
    let and_short = ConditionExpr::compound(
        ConditionExpr::identifier("isOwner"),
        LogicalOperator::And,
        ConditionExpr::identifier("isAdmin"),
    );
    group.bench_function("and_short_circuit", |b| {
        b.iter(|| evaluate(black_box(&and_short), black_box(&state)));
    });

    group.finish();
}

fn expr_negation_bench(c: &mut Criterion) {
    let mut group = c.benchmark_group("expr_negation");

    let state = json!({
        "isDisabled": false,
        "isAdmin": true,
        "status": "active",
    });

    // Simple negation
    let neg_simple = ConditionExpr::negated(ConditionExpr::identifier("isDisabled"));
    group.bench_function("simple", |b| {
        b.iter(|| evaluate(black_box(&neg_simple), black_box(&state)));
    });

    // Negated predicate
    let neg_pred = ConditionExpr::negated(ConditionExpr::predicate(
        "status",
        ComparisonOperator::Equal,
        "'inactive'",
    ));
    group.bench_function("predicate", |b| {
        b.iter(|| evaluate(black_box(&neg_pred), black_box(&state)));
    });

    group.finish();
}

fn expr_realistic_bench(c: &mut Criterion) {
    let mut group = c.benchmark_group("expr_realistic");

    let state = json!({
        "user": {
            "role": "admin",
            "verified": true,
            "suspended": false,
        },
        "item": {
            "state": "done",
            "isActive": true,
        },
        "isLoggedIn": true,
        "hasPermission": true,
    });

    // Real pattern: admin check — compound with predicate + negation
    // (user.role == 'admin') && !user.suspended
    let admin_check = ConditionExpr::compound(
        ConditionExpr::predicate("user.role", ComparisonOperator::Equal, "'admin'"),
        LogicalOperator::And,
        ConditionExpr::negated(ConditionExpr::identifier("user.suspended")),
    );
    group.bench_function("admin_check", |b| {
        b.iter(|| evaluate(black_box(&admin_check), black_box(&state)));
    });

    // Real pattern: auth guard — isLoggedIn && hasPermission
    let auth_guard = ConditionExpr::compound(
        ConditionExpr::identifier("isLoggedIn"),
        LogicalOperator::And,
        ConditionExpr::identifier("hasPermission"),
    );
    group.bench_function("auth_guard", |b| {
        b.iter(|| evaluate(black_box(&auth_guard), black_box(&state)));
    });

    // Real pattern: item state check — item.state == 'done'
    let state_check = ConditionExpr::predicate("item.state", ComparisonOperator::Equal, "'done'");
    group.bench_function("state_check", |b| {
        b.iter(|| evaluate(black_box(&state_check), black_box(&state)));
    });

    group.finish();
}

/// Number of distinct expressions a page-sized workload evaluates per render.
const SITE_EXPRESSION_COUNT: usize = 1000;

/// Cardinality of the generated flag/item/counter fan-out in the site state.
const SITE_FANOUT: usize = 64;

/// Builds a state tree shaped like a real application: a few well-known
/// top-level objects plus wide fan-out collections so generated expressions
/// resolve genuinely different paths instead of hammering one cache line.
fn site_state() -> Value {
    let mut flags = Map::with_capacity(SITE_FANOUT);
    let mut items = Map::with_capacity(SITE_FANOUT);
    let mut counters = Map::with_capacity(SITE_FANOUT);

    for index in 0..SITE_FANOUT {
        flags.insert(format!("f{index}"), Value::Bool(index % 3 != 0));
        items.insert(
            format!("i{index}"),
            json!({
                "state": if index % 4 == 0 { "done" } else { "pending" },
                "priority": index % 5,
                "archived": index % 7 == 0,
                "label": if index % 2 == 0 { "alpha" } else { "beta" },
            }),
        );
        counters.insert(format!("c{index}"), Value::from(index as i64));
    }

    json!({
        "site": { "name": "Contoso", "locale": "en-US", "theme": "dark" },
        "user": {
            "role": "admin",
            "verified": true,
            "suspended": false,
            "profile": { "displayName": "Alice", "tier": "gold", "age": 34, "score": 91.5 },
            "permissions": { "canEdit": true, "canDelete": false, "canPublish": true },
        },
        "session": { "isLoggedIn": true, "expiresIn": 3600, "impersonating": false },
        "cart": { "count": 3, "total": 129.99, "empty": false },
        "flags": flags,
        "items": items,
        "counters": counters,
    })
}

fn flag_path(index: usize) -> String {
    format!("flags.f{}", index % SITE_FANOUT)
}

fn item_path(index: usize, field: &str) -> String {
    format!("items.i{}.{field}", index % SITE_FANOUT)
}

fn counter_path(index: usize) -> String {
    format!("counters.c{}", index % SITE_FANOUT)
}

/// Generates a mixed workload spanning every expression shape a template can
/// produce: shallow and deeply dotted identifiers, string and numeric
/// predicates, variable-to-variable comparisons, two- and three-term compounds
/// that both short-circuit and fully evaluate, and negations at several depths.
fn site_expressions() -> Vec<ConditionExpr> {
    let mut expressions = Vec::with_capacity(SITE_EXPRESSION_COUNT);

    for index in 0..SITE_EXPRESSION_COUNT {
        let next = index + 1;
        let condition = match index % 20 {
            // Identifier truthiness across shallow, two-level and three-level paths.
            0 | 1 => ConditionExpr::identifier(&flag_path(index)),
            2 => ConditionExpr::identifier(&item_path(index, "archived")),
            3 => ConditionExpr::identifier("session.isLoggedIn"),
            4 => ConditionExpr::identifier("user.permissions.canEdit"),

            // Predicates: string equality/inequality and numeric ordering.
            5 => ConditionExpr::predicate(
                &item_path(index, "state"),
                ComparisonOperator::Equal,
                "'done'",
            ),
            6 => ConditionExpr::predicate(
                &item_path(index, "label"),
                ComparisonOperator::NotEqual,
                "'beta'",
            ),
            7 => ConditionExpr::predicate("user.profile.tier", ComparisonOperator::Equal, "'gold'"),
            8 => ConditionExpr::predicate(
                &item_path(index, "priority"),
                ComparisonOperator::GreaterThan,
                "2",
            ),
            9 => ConditionExpr::predicate(
                &counter_path(index),
                ComparisonOperator::LessThanOrEqual,
                "32",
            ),
            10 => ConditionExpr::predicate(
                &counter_path(index),
                ComparisonOperator::LessThan,
                &item_path(next, "priority"),
            ),

            // Two-term compounds, mixing short-circuit and full evaluation.
            11 => ConditionExpr::compound(
                ConditionExpr::identifier("session.isLoggedIn"),
                LogicalOperator::And,
                ConditionExpr::identifier(&flag_path(index)),
            ),
            12 => ConditionExpr::compound(
                ConditionExpr::identifier(&flag_path(index)),
                LogicalOperator::And,
                ConditionExpr::identifier(&item_path(next, "archived")),
            ),
            13 => ConditionExpr::compound(
                ConditionExpr::identifier(&flag_path(index)),
                LogicalOperator::Or,
                ConditionExpr::identifier(&flag_path(next)),
            ),

            // Three-term compounds (two logical operators, homogeneous).
            14 => ConditionExpr::compound(
                ConditionExpr::compound(
                    ConditionExpr::identifier("session.isLoggedIn"),
                    LogicalOperator::And,
                    ConditionExpr::identifier("user.verified"),
                ),
                LogicalOperator::And,
                ConditionExpr::identifier(&flag_path(index)),
            ),
            15 => ConditionExpr::compound(
                ConditionExpr::compound(
                    ConditionExpr::identifier(&flag_path(index)),
                    LogicalOperator::Or,
                    ConditionExpr::identifier(&item_path(index, "archived")),
                ),
                LogicalOperator::Or,
                ConditionExpr::identifier("user.verified"),
            ),

            // Negations, standalone and nested inside compounds.
            16 => ConditionExpr::negated(ConditionExpr::identifier(&item_path(index, "archived"))),
            17 => ConditionExpr::negated(ConditionExpr::predicate(
                &item_path(index, "state"),
                ComparisonOperator::Equal,
                "'pending'",
            )),
            18 => ConditionExpr::compound(
                ConditionExpr::predicate("user.role", ComparisonOperator::Equal, "'admin'"),
                LogicalOperator::And,
                ConditionExpr::negated(ConditionExpr::identifier("user.suspended")),
            ),
            _ => ConditionExpr::compound(
                ConditionExpr::predicate(
                    &item_path(index, "priority"),
                    ComparisonOperator::GreaterThanOrEqual,
                    "1",
                ),
                LogicalOperator::And,
                ConditionExpr::predicate(&counter_path(next), ComparisonOperator::LessThan, "48"),
            ),
        };

        expressions.push(condition);
    }

    expressions
}

/// Deeply dotted lookups only — isolates path resolution cost from tree walking.
fn dotted_expressions() -> Vec<ConditionExpr> {
    (0..SITE_EXPRESSION_COUNT)
        .map(|index| match index % 4 {
            0 => ConditionExpr::identifier(&item_path(index, "archived")),
            1 => ConditionExpr::identifier(&item_path(index, "label")),
            2 => ConditionExpr::predicate(
                &item_path(index, "state"),
                ComparisonOperator::Equal,
                "'done'",
            ),
            _ => ConditionExpr::predicate(
                &item_path(index, "priority"),
                ComparisonOperator::GreaterThan,
                "1",
            ),
        })
        .collect()
}

/// Multi-operator trees only — isolates the continuation-stack traversal cost.
fn compound_expressions() -> Vec<ConditionExpr> {
    (0..SITE_EXPRESSION_COUNT)
        .map(|index| {
            let next = index + 1;
            ConditionExpr::compound(
                ConditionExpr::compound(
                    ConditionExpr::identifier(&flag_path(index)),
                    LogicalOperator::And,
                    ConditionExpr::negated(ConditionExpr::identifier(&item_path(
                        index, "archived",
                    ))),
                ),
                LogicalOperator::And,
                ConditionExpr::predicate(
                    &item_path(next, "priority"),
                    ComparisonOperator::GreaterThanOrEqual,
                    "1",
                ),
            )
        })
        .collect()
}

fn evaluate_all(expressions: &[ConditionExpr], state: &Value) -> usize {
    let mut truthy = 0;
    for condition in expressions {
        if evaluate(condition, state).unwrap_or(false) {
            truthy += 1;
        }
    }
    truthy
}

/// Guards benchmark validity: every generated expression must resolve against
/// the state, otherwise the measured loop would be timing the error path.
fn assert_workload_is_valid(expressions: &[ConditionExpr], state: &Value, label: &str) {
    let failures = expressions
        .iter()
        .filter(|condition| evaluate(condition, state).is_err())
        .count();
    assert_eq!(failures, 0, "{label}: {failures} expressions failed");

    // A workload that is uniformly true or false would let the branch
    // predictor hide real per-expression cost.
    let truthy = evaluate_all(expressions, state);
    assert!(
        truthy > 0 && truthy < expressions.len(),
        "{label}: expected a mix of outcomes, got {truthy}/{}",
        expressions.len()
    );
}

/// Page-sized workload: a single render evaluating a thousand distinct
/// conditions, which is where per-expression overhead compounds into real
/// request latency.
fn expr_site_workload_bench(c: &mut Criterion) {
    let mut group = c.benchmark_group("expr_site_workload");
    group.throughput(Throughput::Elements(SITE_EXPRESSION_COUNT as u64));

    let state = site_state();

    let mixed = site_expressions();
    assert_workload_is_valid(&mixed, &state, "mixed_1000");
    group.bench_function("mixed_1000", |b| {
        b.iter(|| evaluate_all(black_box(&mixed), black_box(&state)));
    });

    let dotted = dotted_expressions();
    assert_workload_is_valid(&dotted, &state, "dotted_1000");
    group.bench_function("dotted_1000", |b| {
        b.iter(|| evaluate_all(black_box(&dotted), black_box(&state)));
    });

    let compound = compound_expressions();
    assert_workload_is_valid(&compound, &state, "compound_1000");
    group.bench_function("compound_1000", |b| {
        b.iter(|| evaluate_all(black_box(&compound), black_box(&state)));
    });

    group.finish();
}

criterion_group!(
    benches,
    expr_identifier_bench,
    expr_predicate_bench,
    expr_compound_bench,
    expr_negation_bench,
    expr_realistic_bench,
    expr_site_workload_bench
);
criterion_main!(benches);
