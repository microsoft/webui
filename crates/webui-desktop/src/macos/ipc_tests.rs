// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use super::*;

fn owner() -> crate::ipc::IpcWindowOwner {
    crate::ipc::IpcWindowOwner::new(
        std::sync::Arc::new(crate::ipc::IpcRegistry::default()),
        crate::ipc::IpcOptions::default(),
        crate::ipc::IpcHost::Source {
            origin: super::super::APP_ORIGIN.into(),
        },
    )
    .unwrap()
}

#[test]
fn canonical_origin_rejects_lookalikes_credentials_and_ports() {
    for input in [
        "webui://evil/",
        "webui://app.evil/",
        "webui://app:9/",
        "webui://user@app/",
        "https://app/",
        "about:blank",
    ] {
        let url = NSURL::URLWithString(&NSString::from_str(input)).unwrap();
        assert!(!trusted_url(&url), "{input}");
    }
    assert!(trusted_url(
        &NSURL::URLWithString(&NSString::from_str("webui://app/path?q=x")).unwrap()
    ));
}

#[test]
fn late_same_url_probes_and_proofs_cannot_activate_a_replacement() {
    let owner = owner();
    let state = MacIpc::new(owner.bridge());
    state.commit_for_test();
    let old = DocumentActivation {
        navigation: state.navigation.get(),
        document_nonce: [1; 16],
        challenge: [2; 16],
    };
    *state.proof.borrow_mut() = Some(old.clone());
    assert!(state.accepts_proof(&old));
    state.commit_for_test();
    assert!(!state.is_current(old.navigation));
    assert!(!state.accepts_proof(&old));
    let new = DocumentActivation {
        navigation: state.navigation.get(),
        document_nonce: [3; 16],
        challenge: [4; 16],
    };
    *state.proof.borrow_mut() = Some(new.clone());
    let forged = DocumentActivation {
        navigation: new.navigation,
        ..old
    };
    assert!(!state.accepts_proof(&forged));
    assert!(state.accepts_proof(&new));
    state.navigate();
    // A cancelled replacement never resurrects its surviving old proof.
    assert!(!state.accepts_proof(&new));
    state.close();
}

#[test]
fn trusted_navigation_publishes_outgoing_retirement_before_cancelling_responses() {
    struct Completion(Rc<RefCell<Vec<&'static str>>>);
    impl Drop for Completion {
        fn drop(&mut self) {
            self.0.borrow_mut().push("response-cancelled");
        }
    }

    let owner = owner();
    let state = MacIpc::new(owner.bridge());
    state.commit_for_test();
    let proof = DocumentActivation {
        navigation: state.navigation.get(),
        document_nonce: [1; 16],
        challenge: [2; 16],
    };
    *state.proof.borrow_mut() = Some(proof.clone());
    *state.session.borrow_mut() = Some(SessionInfo {
        generation: 7,
        token: "a".repeat(32),
        limits: crate::ipc::IpcLimits::default(),
    });
    let order = Rc::new(RefCell::new(Vec::new()));
    let completion = Completion(Rc::clone(&order));
    let tasks = Rc::clone(&state.tasks.borrow());
    tasks
        .spawn(async move {
            let _completion = completion;
            std::future::pending::<()>().await;
        })
        .unwrap();

    state.navigation_started_with(|outgoing, generation| {
        assert_eq!(outgoing, proof);
        assert_eq!(generation, 7);
        assert!(state.is_current(proof.navigation));
        let script = crate::native_ipc::control_script(
            &outgoing,
            NativeControl::Closed {
                generation,
                code: IpcErrorCode::Navigated,
            },
        )
        .unwrap();
        assert!(script.contains("\"code\":\"navigated\""));
        assert!(!script.contains("challenge"));
        order.borrow_mut().push("retirement-queued");
    });
    assert_eq!(*order.borrow(), ["retirement-queued", "response-cancelled"]);
    assert!(!state.is_current(proof.navigation));
    assert!(state.proof.borrow().is_none());
    assert!(state.session.borrow().is_none());
    assert_eq!(
        tasks.spawn(async {}).unwrap_err().code,
        IpcErrorCode::Closed
    );

    let duplicate = Cell::new(false);
    state.navigation_started_with(|_, _| duplicate.set(true));
    assert!(!duplicate.get());
    state.close();
}
