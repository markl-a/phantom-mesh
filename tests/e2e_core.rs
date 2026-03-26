mod common;

#[test]
fn test_common_module_loads() {
    // Verify the common module compiles
    let _msg = common::fixtures::user_msg("hello");
}

use common::harness::CoreHarness;
use clawtex_core::providers::mock::MockProvider;

#[tokio::test]
async fn test_core_harness_basic() {
    let harness = CoreHarness::builder()
        .provider(MockProvider::fixed("Hello from mock!"))
        .build()
        .await;

    let result = harness.run_agent("Say hello").await.unwrap();
    assert!(result.output.contains("Hello from mock"));
    assert_eq!(harness.provider_call_count(), 1);
}
