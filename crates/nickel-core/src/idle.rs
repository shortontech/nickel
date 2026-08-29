use std::time::Duration;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct IdlePolicy {
    pub dim_after: Option<Duration>,
    pub lock_after: Option<Duration>,
    pub suspend_after: Option<Duration>,
}

impl IdlePolicy {
    pub fn from_seconds(dim: Option<u32>, lock: Option<u32>, suspend: Option<u32>) -> Self {
        Self {
            dim_after: dim.map(|seconds| Duration::from_secs(u64::from(seconds))),
            lock_after: lock.map(|seconds| Duration::from_secs(u64::from(seconds))),
            suspend_after: suspend.map(|seconds| Duration::from_secs(u64::from(seconds))),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IdleEffect {
    Dim,
    Undim,
    Lock,
    Suspend,
}

#[derive(Clone, Debug)]
pub struct IdleController {
    policy: IdlePolicy,
    active_at: Duration,
    dimmed: bool,
    lock_requested: bool,
    suspend_requested: bool,
}

impl IdleController {
    pub fn new(policy: IdlePolicy, now: Duration) -> Self {
        Self {
            policy,
            active_at: now,
            dimmed: false,
            lock_requested: false,
            suspend_requested: false,
        }
    }

    pub fn note_activity(&mut self, now: Duration) -> Option<IdleEffect> {
        self.active_at = now;
        self.lock_requested = false;
        self.suspend_requested = false;
        self.dimmed.then(|| {
            self.dimmed = false;
            IdleEffect::Undim
        })
    }

    pub fn poll(
        &mut self,
        now: Duration,
        inhibited: bool,
        already_locked: bool,
    ) -> Vec<IdleEffect> {
        if inhibited {
            return self.note_activity(now).into_iter().collect();
        }

        let elapsed = now.saturating_sub(self.active_at);
        let mut effects = Vec::new();
        if !already_locked
            && !self.dimmed
            && self.policy.dim_after.is_some_and(|after| elapsed >= after)
        {
            self.dimmed = true;
            effects.push(IdleEffect::Dim);
        }
        if !already_locked
            && !self.lock_requested
            && self.policy.lock_after.is_some_and(|after| elapsed >= after)
        {
            self.lock_requested = true;
            effects.push(IdleEffect::Lock);
        }
        if !self.suspend_requested
            && self
                .policy
                .suspend_after
                .is_some_and(|after| elapsed >= after)
        {
            self.suspend_requested = true;
            effects.push(IdleEffect::Suspend);
        }
        effects
    }
}

#[cfg(test)]
mod tests {
    use super::{IdleController, IdleEffect, IdlePolicy};
    use std::time::Duration;

    fn seconds(value: u64) -> Duration {
        Duration::from_secs(value)
    }

    #[test]
    fn transitions_fire_once_in_policy_order() {
        let mut idle = IdleController::new(
            IdlePolicy::from_seconds(Some(10), Some(20), Some(30)),
            Duration::ZERO,
        );

        assert!(idle.poll(seconds(9), false, false).is_empty());
        assert_eq!(idle.poll(seconds(10), false, false), [IdleEffect::Dim]);
        assert!(idle.poll(seconds(19), false, false).is_empty());
        assert_eq!(idle.poll(seconds(20), false, false), [IdleEffect::Lock]);
        assert_eq!(idle.poll(seconds(30), false, true), [IdleEffect::Suspend]);
        assert!(idle.poll(seconds(40), false, true).is_empty());
    }

    #[test]
    fn activity_undims_and_restarts_every_deadline() {
        let mut idle = IdleController::new(
            IdlePolicy::from_seconds(Some(10), Some(20), Some(30)),
            Duration::ZERO,
        );
        assert_eq!(
            idle.poll(seconds(25), false, false),
            [IdleEffect::Dim, IdleEffect::Lock]
        );

        assert_eq!(idle.note_activity(seconds(25)), Some(IdleEffect::Undim));
        assert!(idle.poll(seconds(34), false, false).is_empty());
        assert_eq!(idle.poll(seconds(35), false, false), [IdleEffect::Dim]);
    }

    #[test]
    fn inhibition_undims_and_excludes_inhibited_time() {
        let mut idle = IdleController::new(
            IdlePolicy::from_seconds(Some(10), Some(20), Some(30)),
            Duration::ZERO,
        );
        assert_eq!(idle.poll(seconds(10), false, false), [IdleEffect::Dim]);
        assert_eq!(idle.poll(seconds(15), true, false), [IdleEffect::Undim]);
        assert!(idle.poll(seconds(24), false, false).is_empty());
        assert_eq!(idle.poll(seconds(25), false, false), [IdleEffect::Dim]);
    }

    #[test]
    fn disabled_transitions_never_fire() {
        let mut idle =
            IdleController::new(IdlePolicy::from_seconds(None, None, None), Duration::ZERO);
        assert!(idle.poll(seconds(u32::MAX.into()), false, false).is_empty());
    }
}
