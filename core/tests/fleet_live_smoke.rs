//! Live smoke: requires z13 with the 4 CLIs + a real satellite checkout. Run explicitly:
//!   cargo test --test fleet_live_smoke -- --ignored --nocapture
#[tokio::test]
#[ignore]
async fn one_real_satellite_task_end_to_end() {
    let q = phantom_mesh::fleet::queue::FleetQueue::open_in_memory().unwrap();
    let cfg = phantom_mesh::fleet::FleetConfig::load().unwrap();
    let n = phantom_mesh::fleet::ingest_all(&cfg, &q).await.unwrap();
    println!("ingested {n} tasks from fleet.toml");
    let n2 = phantom_mesh::fleet::ingest_all(&cfg, &q).await.unwrap();
    assert_eq!(
        n2, 0,
        "re-ingesting the same backlog must add 0 new tasks (idempotent)"
    );
}
