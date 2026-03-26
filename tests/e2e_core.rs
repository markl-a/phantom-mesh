mod common;

#[test]
fn test_common_module_loads() {
    // Verify the common module compiles
    let _msg = common::fixtures::user_msg("hello");
}
