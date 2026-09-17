# microsoft-webui-expressions

Expression evaluator for the [WebUI](https://github.com/microsoft/webui) framework. Evaluates template binding expressions against JSON state at render time.

## Overview

`microsoft-webui-expressions` provides a fast, allocation-conscious expression engine for resolving data bindings, conditional expressions, and loop iterators inside WebUI templates.

## Repeated condition evaluation

Use `PreparedCondition` when evaluating the same condition repeatedly. Prepare it
once when loading a protocol, then reuse it across renders and states:

```rust
use webui_expressions::PreparedCondition;
use webui_protocol::{ComparisonOperator, ConditionExpr};

let condition = ConditionExpr::predicate("status", ComparisonOperator::Equal, "'active'");
let prepared = PreparedCondition::new(&condition);
let state = serde_json::json!({ "status": "active" });

assert_eq!(prepared.evaluate(&state).ok(), Some(true));
```

`new(&ConditionExpr) -> PreparedCondition` is infallible. Evaluation returns
`Result<bool, ExpressionError>`, including errors cached during preparation.
Short-circuited malformed subexpressions remain skipped. The global five-logical-
operator limit and mixed-operator check apply before any value lookup, including
operators beneath negation.

`evaluate_with_resolver` accepts
`Fn(&str) -> Option<Cow<'a, serde_json::Value>>`, preserving borrowed state
lookups and supporting owned synthetic values without cloning a state tree.
Missing identifiers are false; missing comparison operands are errors. Boolean
identifier constants `true` and `false` never invoke the resolver.

A prepared condition owns its paths and literal values independently of the
original condition, and can be shared across threads. Repeated evaluation avoids
tree validation, traversal allocations, and right-hand literal parsing.
Preparation adds one-time work and retained storage; the existing `evaluate`
and `evaluate_with_resolver` functions remain available for one-off evaluation.
Resolver-created values and returned errors can still allocate.

## Benchmarks

The existing `expr_*` benchmark groups exercise direct evaluation. The
`prepared_*` groups use the exact same conditions and state, preparing outside
the timed loop. `prepare_admin_check` measures preparation separately.

```bash
cargo bench -p microsoft-webui-expressions --bench expressions_bench -- --save-baseline current
```

Compare release-mode runs with an idle machine; retain both latency measurements
and the cost of preparing and retaining conditions when deciding where to reuse
them.

## Documentation

See the [WebUI repository](https://github.com/microsoft/webui) for full usage guides and examples.

## License

MIT — Copyright (c) Microsoft Corporation.
