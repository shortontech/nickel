//! Replaceable system state, deliberately separate from ordered command queues.
//!
//! A slow consumer retains at most one snapshot per domain. Wake callbacks run
//! outside the slot lock; draining clears pending state under that same lock so
//! a concurrent publisher either joins this drain or schedules the next wake.

use std::sync::{Arc, Condvar, Mutex, Weak};

use super::SystemStatusUpdate;

type Wake = Arc<dyn Fn() + Send + Sync>;

#[derive(Default)]
struct Pending {
    slots: [Option<Arc<SystemStatusUpdate>>; 4],
    wake: Option<Wake>,
    audio_activity: super::AudioActivity,
}

#[derive(Default)]
struct Shared {
    pending: Mutex<Pending>,
    ready: Condvar,
}

#[derive(Clone)]
pub(crate) struct StatusSender(Weak<Shared>);

pub(crate) struct StatusReceiver {
    shared: Arc<Shared>,
    cleanup: Vec<Box<dyn FnOnce() + Send>>,
    resources: Vec<Box<dyn Send>>,
}

pub(crate) fn channel() -> (StatusSender, StatusReceiver) {
    let shared = Arc::new(Shared::default());
    (
        StatusSender(Arc::downgrade(&shared)),
        StatusReceiver {
            shared,
            cleanup: Vec::new(),
            resources: Vec::new(),
        },
    )
}

impl StatusSender {
    pub(crate) fn same_channel(&self, other: &Self) -> bool {
        Weak::ptr_eq(&self.0, &other.0)
    }

    pub(crate) fn send(&self, update: Arc<SystemStatusUpdate>) -> Result<(), ()> {
        let shared = self.0.upgrade().ok_or(())?;
        let slot = match update.as_ref() {
            SystemStatusUpdate::Audio(_) | SystemStatusUpdate::AudioWithActivity { .. } => 0,
            SystemStatusUpdate::Network(_) => 1,
            SystemStatusUpdate::Bluetooth(_) => 2,
            SystemStatusUpdate::ShellSettingsChanged => 3,
        };
        let (retired, wake) = {
            let mut pending = shared.pending.lock().unwrap();
            let needs_wake = pending.slots.iter().all(Option::is_none);
            // Keep feedback facts, not intermediate device lists. A volume/mute
            // round trip still deserves feedback, but an availability round trip
            // must not make a reconnect look like a user volume adjustment.
            if let Some((next, incoming)) = audio_state(update.as_ref()) {
                if let Some((previous, _)) = pending.slots[0].as_deref().and_then(audio_state) {
                    let availability_changed = previous.available != next.available;
                    let value_changed = previous.available
                        && next.available
                        && (previous.volume_percent != next.volume_percent
                            || previous.muted != next.muted);
                    if availability_changed {
                        pending.audio_activity.value_changed = false;
                    }
                    pending.audio_activity.availability_changed |= availability_changed;
                    pending.audio_activity.value_changed |= value_changed;
                }
                pending.audio_activity.availability_changed |= incoming.availability_changed;
                if incoming.availability_changed {
                    pending.audio_activity.value_changed = false;
                }
                pending.audio_activity.value_changed |= incoming.value_changed;
            }
            let retired = pending.slots[slot].replace(update);
            (retired, needs_wake.then(|| pending.wake.clone()).flatten())
        };
        // Device strings/vectors may be large: retire them outside the critical
        // section, just like the wake callback. No producer waits for UI work.
        drop(retired);
        shared.ready.notify_one();
        if let Some(wake) = wake {
            wake();
        }
        Ok(())
    }
}

impl StatusReceiver {
    pub(crate) fn sender(&self) -> StatusSender {
        StatusSender(Arc::downgrade(&self.shared))
    }

    pub(crate) fn on_drop(&mut self, cleanup: impl FnOnce() + Send + 'static) {
        self.cleanup.push(Box::new(cleanup));
    }

    pub(crate) fn keep_alive(&mut self, resource: impl Send + 'static) {
        self.resources.push(Box::new(resource));
    }

    pub(crate) fn set_waker(&self, wake: impl Fn() + Send + Sync + 'static) {
        let wake: Wake = Arc::new(wake);
        let pending = {
            let mut pending = self.shared.pending.lock().unwrap();
            pending.wake = Some(wake.clone());
            pending.slots.iter().any(Option::is_some)
        };
        // Registering after startup snapshots were published must not lose them.
        if pending {
            wake();
        }
    }

    pub(crate) fn drain(&self) -> Vec<SystemStatusUpdate> {
        let (slots, activity) = {
            let mut pending = self.shared.pending.lock().unwrap();
            (
                std::mem::take(&mut pending.slots),
                std::mem::take(&mut pending.audio_activity),
            )
        };
        slots
            .into_iter()
            .flatten()
            .map(Arc::unwrap_or_clone)
            .map(|update| match update {
                SystemStatusUpdate::Audio(status)
                | SystemStatusUpdate::AudioWithActivity { status, .. }
                    if activity != super::AudioActivity::default() =>
                {
                    SystemStatusUpdate::AudioWithActivity { status, activity }
                }
                update => update,
            })
            .collect()
    }

    /// Blocking adapter for the existing external audio feed only. Native shell
    /// delivery uses drain plus a coalesced event-loop wake, without relay threads.
    pub(crate) fn recv(&self) -> SystemStatusUpdate {
        let mut pending = self.shared.pending.lock().unwrap();
        loop {
            if let Some((index, slot)) = pending
                .slots
                .iter_mut()
                .enumerate()
                .find(|(_, slot)| slot.is_some())
            {
                let update = slot.take().unwrap();
                if index == 0 {
                    pending.audio_activity = Default::default();
                }
                drop(pending);
                return Arc::unwrap_or_clone(update);
            }
            pending = self.shared.ready.wait(pending).unwrap();
        }
    }
}

fn audio_state(update: &SystemStatusUpdate) -> Option<(&super::AudioStatus, super::AudioActivity)> {
    match update {
        SystemStatusUpdate::Audio(status) => Some((status, Default::default())),
        SystemStatusUpdate::AudioWithActivity { status, activity } => Some((status, *activity)),
        _ => None,
    }
}

impl Drop for StatusReceiver {
    fn drop(&mut self) {
        // Remove registrations even when a quiet backend will never publish again.
        for cleanup in self.cleanup.drain(..) {
            cleanup();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn audio(volume_percent: u8) -> Arc<SystemStatusUpdate> {
        Arc::new(SystemStatusUpdate::Audio(super::super::AudioStatus {
            volume_percent,
            ..Default::default()
        }))
    }

    #[test]
    fn stalled_consumer_keeps_latest_domains_and_one_wake() {
        let (sender, receiver) = channel();
        let wakes = Arc::new(AtomicUsize::new(0));
        let count = wakes.clone();
        receiver.set_waker(move || {
            count.fetch_add(1, Ordering::SeqCst);
        });
        let first = audio(1);
        let retired = Arc::downgrade(&first);
        sender.send(first).unwrap();
        for index in 0..10_000 {
            sender.send(audio((index % 101) as u8)).unwrap();
        }
        sender
            .send(Arc::new(SystemStatusUpdate::Network(Default::default())))
            .unwrap();
        sender
            .send(Arc::new(SystemStatusUpdate::Bluetooth(Default::default())))
            .unwrap();
        for _ in 0..1_000 {
            sender
                .send(Arc::new(SystemStatusUpdate::ShellSettingsChanged))
                .unwrap();
        }
        assert!(retired.upgrade().is_none());
        assert_eq!(wakes.load(Ordering::SeqCst), 1);
        assert_eq!(
            receiver
                .shared
                .pending
                .lock()
                .unwrap()
                .slots
                .iter()
                .flatten()
                .count(),
            4
        );
        let updates = receiver.drain();
        assert_eq!(updates.len(), 4);
        assert!(updates.contains(&*audio((9_999 % 101) as u8)));
        assert!(receiver.drain().is_empty());
        sender.send(audio(10)).unwrap();
        assert_eq!(wakes.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn registering_wake_after_publish_and_reentrant_publish_do_not_deadlock() {
        let (sender, receiver) = channel();
        sender.send(audio(1)).unwrap();
        let nested = sender.clone();
        receiver.set_waker(move || {
            nested.send(audio(2)).unwrap();
        });
        assert_eq!(receiver.drain(), vec![(*audio(2)).clone()]);
    }

    #[test]
    fn drop_retires_registration_without_waiting_for_backend_publish() {
        let (sender, mut receiver) = channel();
        let registry = Arc::new(Mutex::new(vec![sender.clone()]));
        let cleanup_registry = registry.clone();
        let cleanup_sender = sender.clone();
        receiver.on_drop(move || {
            cleanup_registry
                .lock()
                .unwrap()
                .retain(|entry| !entry.same_channel(&cleanup_sender))
        });
        let payload = audio(7);
        let retired = Arc::downgrade(&payload);
        sender.send(payload).unwrap();
        drop(receiver);
        assert!(registry.lock().unwrap().is_empty());
        assert!(retired.upgrade().is_none());
        assert!(sender.send(audio(8)).is_err());
    }

    #[test]
    fn subscribers_share_pending_payload_and_retire_it_independently() {
        let (first, first_receiver) = channel();
        let (second, second_receiver) = channel();
        let update = audio(17);
        let weak = Arc::downgrade(&update);
        first.send(update.clone()).unwrap();
        second.send(update).unwrap();
        assert_eq!(weak.strong_count(), 2);
        drop(first_receiver);
        assert_eq!(weak.strong_count(), 1);
        assert_eq!(second_receiver.drain(), vec![(*audio(17)).clone()]);
        assert!(weak.upgrade().is_none());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn calloop_wake_delivers_latest_state_and_rearms() {
        use smithay::reexports::calloop::{EventLoop, ping::make_ping};
        let mut event_loop = EventLoop::<Vec<SystemStatusUpdate>>::try_new().unwrap();
        let (sender, receiver) = channel();
        let (ping, source) = make_ping().unwrap();
        receiver.set_waker(move || ping.ping());
        let token = event_loop
            .handle()
            .insert_source(source, move |_, _, updates| {
                updates.extend(receiver.drain())
            })
            .unwrap();
        for volume in 0..100 {
            sender.send(audio(volume)).unwrap();
        }
        let mut updates = Vec::new();
        event_loop
            .dispatch(std::time::Duration::ZERO, &mut updates)
            .unwrap();
        assert_eq!(updates, vec![(*audio(99)).clone()]);
        sender.send(audio(20)).unwrap();
        event_loop
            .dispatch(std::time::Duration::ZERO, &mut updates)
            .unwrap();
        assert_eq!(updates.last(), Some(audio(20).as_ref()));
        event_loop.handle().remove(token);
        assert!(sender.send(audio(21)).is_err());
    }

    #[test]
    fn publish_racing_drain_delivers_final_state() {
        let (sender, receiver) = channel();
        let worker = std::thread::spawn(move || {
            for volume in 0..100 {
                sender.send(audio(volume)).unwrap();
            }
        });
        let mut latest = None;
        while !worker.is_finished() {
            for update in receiver.drain() {
                latest = Some(update);
            }
            std::thread::yield_now();
        }
        worker.join().unwrap();
        for update in receiver.drain() {
            latest = Some(update);
        }
        assert_eq!(latest, Some((*audio(99)).clone()));
    }
}
