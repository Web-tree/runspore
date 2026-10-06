//! The crate's public surface: `describe`, `transition`, and `NativeReducer`.

mod common;

use common::*;
use runspore_kernel::{describe, transition, NativeReducer};
use runspore_types::model::{ABI_VERSION, CODEC_VERSION, SEMANTICS_VERSION, WORKFLOW_FORMAT};
use runspore_types::reducer::{Descriptor, Reducer};
use serde_json::json;

#[test]
fn describe_reports_the_frozen_versions() {
    assert_eq!(
        describe(),
        Descriptor {
            abi_version: ABI_VERSION.to_string(),
            semantics_version: SEMANTICS_VERSION.to_string(),
            graph_format_version: WORKFLOW_FORMAT.to_string(),
            codec_version: CODEC_VERSION.to_string(),
        }
    );
}

#[test]
fn the_native_reducer_is_the_kernel() {
    let reducer: &dyn Reducer = &NativeReducer;
    assert_eq!(reducer.describe(), describe());
    assert_eq!(
        reducer.kernel_digest(),
        format!("native:runspore-kernel@{}", env!("CARGO_PKG_VERSION"))
    );

    let ok = start_request(&review_loop(), json!({"repo": "r"}));
    assert!(transition(&ok).is_ok());
    assert_eq!(reducer.transition(&ok), transition(&ok));

    let mut bad = ok;
    bad.identity.semantics_version = "9.9".to_string();
    assert!(transition(&bad).is_err());
    assert_eq!(reducer.transition(&bad), transition(&bad));
}
