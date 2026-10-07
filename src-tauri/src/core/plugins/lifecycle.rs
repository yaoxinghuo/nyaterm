use crate::error::{AppError, AppResult};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

#[derive(Default)]
struct Activity {
    requests: usize,
    changing: bool,
}

#[derive(Default, Clone)]
pub(super) struct Lifecycle {
    state: Arc<Mutex<HashMap<String, Activity>>>,
}

pub(super) struct Lease {
    lifecycle: Lifecycle,
    id: String,
    mutation: bool,
}

impl Lifecycle {
    pub fn acquire(&self, id: &str, mutation: bool) -> AppResult<Lease> {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let activity = state.entry(id.into()).or_default();
        if activity.changing || (mutation && activity.requests > 0) {
            return Err(AppError::Config(
                "Plugin is busy; wait for its active operations to finish.".into(),
            ));
        }
        if mutation {
            activity.changing = true;
        } else {
            activity.requests += 1;
        }
        Ok(Lease {
            lifecycle: self.clone(),
            id: id.into(),
            mutation,
        })
    }
}

impl Drop for Lease {
    fn drop(&mut self) {
        let mut state = self
            .lifecycle
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(activity) = state.get_mut(&self.id) {
            if self.mutation {
                activity.changing = false;
            } else {
                activity.requests = activity.requests.saturating_sub(1);
            }
            if !activity.changing && activity.requests == 0 {
                state.remove(&self.id);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mutations_and_requests_are_admitted_atomically_per_plugin() {
        let lifecycle = Lifecycle::default();
        let request = lifecycle.acquire("a", false).unwrap();
        assert!(lifecycle.acquire("a", true).is_err());
        let other = lifecycle.acquire("b", true).unwrap();
        assert!(lifecycle.acquire("b", false).is_err());
        drop(request);
        let update = lifecycle.acquire("a", true).unwrap();
        assert!(lifecycle.acquire("a", false).is_err());
        drop(update);
        drop(other);
        assert!(lifecycle.acquire("a", false).is_ok());
    }
}
