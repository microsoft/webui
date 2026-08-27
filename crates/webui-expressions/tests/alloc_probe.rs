// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Guards the "no allocation on the valid path" contract in DESIGN.md.
//!
//! A counting global allocator is the only way to observe this directly, and
//! implementing `GlobalAlloc` requires `unsafe`. The allow is scoped to this
//! test binary and the allocator only increments a counter before delegating
//! to `System`, so it introduces no new invariants of its own.
#![allow(unsafe_code)]

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};

use serde_json::json;
use webui_expressions::evaluate;
use webui_protocol::{ComparisonOperator, ConditionExpr, LogicalOperator};

static ALLOCS: AtomicUsize = AtomicUsize::new(0);

struct Counting;

// SAFETY: `Counting` adds a relaxed counter increment and otherwise forwards
// every call unchanged to `System`, so it inherits `System`'s guarantees and
// introduces no invariants of its own. The inner `unsafe` blocks are kept
// explicit rather than relying on the edition 2021 rule that an `unsafe fn`
// body is itself an unsafe block, because that rule is removed in edition 2024.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOCS.fetch_add(1, Ordering::Relaxed);
        // SAFETY: `layout` is forwarded unchanged from the caller, who already
        // owes `GlobalAlloc::alloc` a non-zero-sized layout.
        unsafe { System.alloc(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: `ptr` came from `Self::alloc`, which delegates to `System`,
        // and `layout` is the same one used to allocate it.
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static A: Counting = Counting;

fn count(label: &str, condition: &ConditionExpr, state: &serde_json::Value) {
    let _ = evaluate(condition, state);
    let before = ALLOCS.load(Ordering::Relaxed);
    let _ = evaluate(condition, state);
    let after = ALLOCS.load(Ordering::Relaxed);
    assert_eq!(
        after - before,
        0,
        "{label} allocated {} time(s)",
        after - before
    );
}

#[test]
fn valid_conditions_do_not_allocate() {
    let state = json!({"a": true, "b": true, "c": true, "name": "John", "user": {"age": 30}});
    count("identifier", &ConditionExpr::identifier("a"), &state);
    count(
        "string_predicate",
        &ConditionExpr::predicate("name", ComparisonOperator::Equal, "'John'"),
        &state,
    );
    count(
        "dotted_predicate",
        &ConditionExpr::predicate("user.age", ComparisonOperator::GreaterThan, "18"),
        &state,
    );
    let compound = ConditionExpr::compound(
        ConditionExpr::identifier("a"),
        LogicalOperator::And,
        ConditionExpr::compound(
            ConditionExpr::identifier("b"),
            LogicalOperator::And,
            ConditionExpr::predicate("name", ComparisonOperator::Equal, "'John'"),
        ),
    );
    count("compound", &compound, &state);
    count(
        "negation",
        &ConditionExpr::negated(ConditionExpr::identifier("a")),
        &state,
    );
}
