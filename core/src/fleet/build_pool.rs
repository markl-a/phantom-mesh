//! In-process semaphore capping concurrent build/test steps on this node.
use std::sync::Arc;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

#[derive(Clone)]
pub struct BuildPool {
    sem: Arc<Semaphore>,
}

impl BuildPool {
    pub fn new(permits: usize) -> Self {
        BuildPool {
            sem: Arc::new(Semaphore::new(permits)),
        }
    }
    /// Acquire a build permit, awaiting if the pool is full. Held until dropped.
    pub async fn acquire(&self) -> OwnedSemaphorePermit {
        self.sem
            .clone()
            .acquire_owned()
            .await
            .expect("semaphore not closed")
    }
    pub fn available(&self) -> usize {
        self.sem.available_permits()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn permits_are_bounded() {
        let pool = BuildPool::new(2);
        let p1 = pool.acquire().await;
        let _p2 = pool.acquire().await;
        assert_eq!(pool.available(), 0, "both permits taken");
        drop(p1);
        assert_eq!(pool.available(), 1, "dropping a permit frees a slot");
    }
}
