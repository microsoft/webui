use criterion::{criterion_group, criterion_main, Criterion};
use serde_json::json;
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

criterion_group!(
    benches,
    expr_identifier_bench,
    expr_predicate_bench,
    expr_compound_bench,
    expr_negation_bench,
    expr_realistic_bench
);
criterion_main!(benches);
