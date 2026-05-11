use crate::GoalsStore;

pub fn goals_context(store: &GoalsStore) -> anyhow::Result<String> {
    let goals = store.inner.lock().unwrap();
    if goals.is_empty() {
        return Ok(String::new());
    }
    let mut ctx = String::from("Active goals:\n");
    for g in goals.iter() {
        ctx.push_str(&format!("- {}\n", g));
    }
    Ok(ctx)
}
