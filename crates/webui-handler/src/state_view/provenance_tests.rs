// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

#![allow(clippy::disallowed_methods)]

use super::*;
use webui_test_utils::test_json;

fn assert_policy(value: &SharedValue, policy: Provenance) {
    assert_eq!(value.provenance().is_some(), policy == Provenance::Track);
    assert_eq!(value.provenance_policy(), policy);
}

#[test]
fn optional_provenance_keeps_handle_and_state_headers_unchanged() {
    assert_eq!(
        std::mem::size_of::<SharedValue>(),
        std::mem::size_of::<[usize; 3]>()
    );
    assert_eq!(
        std::mem::size_of::<SharedState>(),
        std::mem::size_of::<(BTreeMap<String, SharedValue>, Option<SharedValue>)>()
    );
}

#[test]
fn both_policies_keep_projected_value_identity_and_release_original_inputs() {
    for policy in [Provenance::Omit, Provenance::Track] {
        let input = test_json!({"rows": [{"child": [1, 2]}]});
        let original_child = &input["rows"][0]["child"] as *const Value;
        let root = SharedValue::with_provenance(input, policy);
        let weak = Arc::downgrade(root.value.backing_cart());
        let rows = root.project("rows").unwrap();
        let row = rows.item(0).unwrap();
        let child = row.project("child").unwrap();
        let cloned = child.clone();
        for value in [&root, &rows, &row, &child, &cloned] {
            assert_policy(value, policy);
            assert!(Arc::ptr_eq(
                root.value.backing_cart(),
                value.value.backing_cart()
            ));
        }
        assert!(std::ptr::eq(child.get(), original_child));
        assert!(rows.item(1).is_none());
        assert!(row.project("missing").is_none());
        let length = child.project("length").unwrap();
        assert_policy(&length, policy);
        assert_eq!(length.get(), 2);
        assert!(!Arc::ptr_eq(
            root.value.backing_cart(),
            length.value.backing_cart()
        ));
        drop((root, rows, row, child));
        assert!(weak.upgrade().is_some());
        drop(cloned);
        assert!(weak.upgrade().is_none());
        assert_eq!(length.get(), 2);
    }
}

#[test]
fn untracked_and_synthetic_projections_never_materialize_path_storage() {
    let root = SharedValue::with_provenance(test_json!({"rows": [1, 2]}), Provenance::Omit);
    let rows = root
        .project_with_path("rows", || panic!("untracked paths must stay borrowed"))
        .unwrap();
    assert_policy(&rows, Provenance::Omit);
    assert_policy(&rows.item(0).unwrap(), Provenance::Omit);
    for policy in [Provenance::Omit, Provenance::Track] {
        let rows = SharedValue::with_provenance(test_json!([1, 2]), policy);
        let length = rows
            .project_with_path("length", || panic!("synthetic values need no path"))
            .unwrap();
        assert_policy(&length, policy);
        assert_eq!(length.get(), 2);
        assert!(rows
            .project_with_path("absent", || panic!("missing values need no path"))
            .is_none());
    }
}

#[test]
fn all_snapshot_constructors_apply_policy_without_changing_move_or_copy_semantics() {
    let keys = [Box::<str>::from("keep"), Box::<str>::from("absent")];
    for policy in [Provenance::Omit, Provenance::Track] {
        for selected in [false, true] {
            for owned in [false, true] {
                let input = test_json!({"keep": {"child": [1, 2]}, "skip": [3]});
                let original_child = &input["keep"]["child"] as *const Value;
                let shared = match (owned, selected) {
                    (true, true) => {
                        SharedState::from_selected_owned_with_provenance(input, &keys, policy)
                    }
                    (true, false) => SharedState::from_owned_with_provenance(input, policy),
                    (false, true) => {
                        SharedState::from_selected_with_provenance(&input, &keys, policy)
                    }
                    (false, false) => SharedState::from_borrowed_with_provenance(&input, policy),
                };
                for value in shared.roots.values() {
                    assert_policy(value, policy);
                }
                let child = shared.capture("keep.child").unwrap();
                assert_policy(&child, policy);
                assert_eq!(std::ptr::eq(child.get(), original_child), owned);
                assert_eq!(shared.get("skip").is_none(), selected);
                assert_eq!(child.get(), &test_json!([1, 2]));
            }
        }
    }
}

#[test]
fn overlays_preserve_old_inputs_and_apply_policy_to_new_and_replaced_roots() {
    let keys = [
        Box::<str>::from("root"),
        Box::<str>::from("same"),
        Box::<str>::from("new"),
    ];
    for policy in [Provenance::Omit, Provenance::Track] {
        for owned in [false, true] {
            for selected in [false, true] {
                let mut state = SharedState::from_owned_with_provenance(
                    test_json!({"root": {"child": [1]}, "same": [3], "skip": [4]}),
                    policy,
                );
                let old = state.capture("root.child").unwrap();
                let old_pointer = old.get() as *const Value;
                let old_root = Arc::downgrade(old.value.backing_cart());
                let same = state.capture("same").unwrap();
                let patch = test_json!({
                    "root": {"child": [2]}, "same": [3], "new": [5], "skip": [6]
                });
                let selection = selected.then_some(keys.as_slice());
                let changed = if owned {
                    state.overlay_owned_with_provenance(patch, selection, policy)
                } else {
                    state.overlay_with_provenance(&patch, selection, policy)
                };
                assert!(changed);
                assert!(std::ptr::eq(old.get(), old_pointer));
                assert_eq!(old.get(), &test_json!([1]));
                assert!(Arc::ptr_eq(
                    same.value.backing_cart(),
                    state.roots["same"].value.backing_cart()
                ));
                for value in state.roots.values() {
                    assert_policy(value, policy);
                }
                let expected_skip = if selected {
                    test_json!([4])
                } else {
                    test_json!([6])
                };
                assert_eq!(state.get("skip"), Some(&expected_skip));
                let unchanged = test_json!({"same": [3]});
                assert!(!if owned {
                    state.overlay_owned_with_provenance(unchanged, None, policy)
                } else {
                    state.overlay_with_provenance(&unchanged, None, policy)
                });
                state.clear();
                assert!(old_root.upgrade().is_some());
                drop(old);
                assert!(old_root.upgrade().is_none());
            }
        }
    }
}

#[test]
fn scalar_and_initially_empty_selected_states_keep_the_explicit_policy() {
    let keys = [Box::<str>::from("new")];
    for policy in [Provenance::Omit, Provenance::Track] {
        for owned in [false, true] {
            let input = test_json!([1, 2]);
            let mut scalar = if owned {
                SharedState::from_owned_with_provenance(input, policy)
            } else {
                SharedState::from_borrowed_with_provenance(&input, policy)
            };
            assert_policy(scalar.scalar.as_ref().unwrap(), policy);
            assert_policy(&scalar.capture("length").unwrap(), policy);
            assert_eq!(
                serde_json::to_value(StateView::shared(&scalar)).unwrap(),
                test_json!([1, 2])
            );
            assert!(!scalar.overlay_with_provenance(&Value::Null, None, policy));
            assert!(!scalar.overlay_owned_with_provenance(Value::Bool(false), None, policy));
            let patch = test_json!({"new": [7]});
            assert!(if owned {
                scalar.overlay_owned_with_provenance(patch, None, policy)
            } else {
                scalar.overlay_with_provenance(&patch, None, policy)
            });
            assert!(scalar.scalar.is_none());
            assert_policy(&scalar.capture("new").unwrap(), policy);
            let mut selected = if owned {
                SharedState::from_selected_owned_with_provenance(Value::Null, &keys, policy)
            } else {
                SharedState::from_selected_with_provenance(&Value::Null, &keys, policy)
            };
            assert!(selected.roots.is_empty());
            let patch = test_json!({"new": [9], "ignored": [8]});
            assert!(if owned {
                selected.overlay_owned_with_provenance(patch, Some(&keys), policy)
            } else {
                selected.overlay_with_provenance(&patch, Some(&keys), policy)
            });
            assert_policy(&selected.capture("new").unwrap(), policy);
            assert!(selected.get("ignored").is_none());
        }
    }
}
