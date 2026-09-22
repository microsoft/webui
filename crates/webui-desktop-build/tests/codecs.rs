// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

#![allow(clippy::disallowed_methods)]

use prost::Message;
use std::collections::BTreeMap;

#[allow(dead_code)]
mod generated {
    include!("../../webui-desktop/tests/fixtures/typed-ipc/rust/ipc_messages.rs");
}
use generated::example::desktop::{item, Item};
use generated::example::types::Metrics;

fn sample() -> Item {
    Item {
        id: u64::MAX,
        image: vec![0, 255, 128, 1],
        title: Some(String::new()),
        minimum: i64::MIN,
        status: 123456,
        scores: vec![i32::MIN, i32::MAX],
        labels: BTreeMap::from([
            ("__proto__".into(), "prototype data".into()),
            ("constructor".into(), "constructor data".into()),
            ("prototype".into(), "ordinary data".into()),
            ("label".into(), "value".into()),
        ]),
        choice: Some(item::Choice::Number(i64::MAX)),
        chunks: BTreeMap::from([(i32::MIN, vec![0, 255])]),
        enabled: Some(false),
        metrics: None,
        flags: BTreeMap::from([(false, "false".into()), (true, "true".into())]),
        unsigned_keys: BTreeMap::from([(0, "zero".into()), (u64::MAX, "maximum".into())]),
        signed_keys: BTreeMap::from([(i64::MIN, "minimum".into()), (i64::MAX, "maximum".into())]),
    }
}

#[test]
fn generated_codecs_preserve_extremes_and_golden_binary() {
    let item = sample();
    let bytes = item.encode_to_vec();
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../webui-desktop/tests/fixtures/typed-ipc/golden.bin");
    if std::env::var_os("WEBUI_UPDATE_IPC_FIXTURE").is_some() {
        std::fs::write(&path, &bytes).unwrap();
    } else {
        let typescript = std::fs::read(path.with_file_name("golden-ts.bin")).unwrap();
        assert_eq!(Item::decode(typescript.as_slice()).unwrap(), item);
    }
    assert_eq!(bytes, std::fs::read(path).unwrap());
    assert_eq!(Item::decode(bytes.as_slice()).unwrap(), item);
    assert_eq!(<()>::decode(&[][..]).unwrap(), ());
    assert!(().encode_to_vec().is_empty());
}

#[test]
fn generated_binary_codec_handles_large_bytes_and_absent_optional() {
    let mut item = sample();
    item.image = vec![0xab; 256 * 1024];
    item.title = None;
    item.enabled = None;
    item.choice = Some(item::Choice::Text("text".into()));
    assert_eq!(Item::decode(item.encode_to_vec().as_slice()).unwrap(), item);
}

#[test]
fn protobuf_nonfinite_and_all_numeric_encodings() {
    let metrics = Metrics {
        infinity: f64::INFINITY,
        not_a_number: f32::NAN,
        maximum_fixed: u64::MAX,
        minimum_fixed: i64::MIN,
        minimum_zigzag: i64::MIN,
        maximum_fixed32: u32::MAX,
        minimum_fixed32: i32::MIN,
        maximum_unsigned: u32::MAX,
        minimum_signed: i32::MIN,
        truth: true,
        details: Vec::new(),
    };
    let decoded = Metrics::decode(metrics.encode_to_vec().as_slice()).unwrap();
    assert!(decoded.not_a_number.is_nan());
    assert_eq!(decoded.infinity, f64::INFINITY);
    assert_eq!(decoded.maximum_fixed, u64::MAX);
    assert_eq!(decoded.minimum_fixed, i64::MIN);
    assert_eq!(decoded.minimum_zigzag, i64::MIN);
    assert_eq!(decoded.maximum_fixed32, u32::MAX);
    assert_eq!(decoded.minimum_fixed32, i32::MIN);
    assert_eq!(decoded.maximum_unsigned, u32::MAX);
    assert_eq!(decoded.minimum_signed, i32::MIN);
}
