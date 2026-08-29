use std::{
    collections::VecDeque,
    time::{Duration, Instant},
};

pub const MAX_NOTIFICATIONS: usize = 100;
pub const MAX_NOTIFICATION_ACTIONS: usize = 3;
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NotificationAction {
    pub key: String,
    pub label: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NotificationRequest {
    pub app_name: String,
    pub summary: String,
    pub body: String,
    pub actions: Vec<NotificationAction>,
    pub expire_timeout_ms: i32,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DesktopNotification {
    pub id: u32,
    pub app_name: String,
    pub summary: String,
    pub body: String,
    pub actions: Vec<NotificationAction>,
    expires_at: Option<Instant>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ClosedNotification {
    pub id: u32,
    pub reason: u32,
}

#[derive(Debug)]
pub struct NotificationStore {
    notifications: VecDeque<DesktopNotification>,
    next_id: u32,
}

impl Default for NotificationStore {
    fn default() -> Self {
        Self {
            notifications: VecDeque::new(),
            next_id: 1,
        }
    }
}

impl NotificationStore {
    pub fn notify(
        &mut self,
        replaces_id: u32,
        request: NotificationRequest,
        now: Instant,
    ) -> (u32, Option<ClosedNotification>) {
        let expires_at = if request.expire_timeout_ms > 0 {
            Some(now + Duration::from_millis(request.expire_timeout_ms as u64))
        } else if request.expire_timeout_ms == 0 {
            None
        } else {
            Some(now + DEFAULT_TIMEOUT)
        };
        if replaces_id != 0
            && let Some(notification) = self
                .notifications
                .iter_mut()
                .find(|notification| notification.id == replaces_id)
        {
            notification.app_name = request.app_name;
            notification.summary = request.summary;
            notification.body = request.body;
            notification.actions = request.actions;
            notification.expires_at = expires_at;
            return (replaces_id, None);
        }

        let id = self.allocate_id();
        self.notifications.push_back(DesktopNotification {
            id,
            app_name: request.app_name,
            summary: request.summary,
            body: request.body,
            actions: request.actions,
            expires_at,
        });
        let discarded = (self.notifications.len() > MAX_NOTIFICATIONS)
            .then(|| self.notifications.pop_front())
            .flatten()
            .map(|notification| ClosedNotification {
                id: notification.id,
                reason: 4,
            });
        (id, discarded)
    }

    pub fn close(&mut self, id: u32, reason: u32) -> Option<ClosedNotification> {
        let index = self
            .notifications
            .iter()
            .position(|notification| notification.id == id)?;
        self.notifications.remove(index);
        Some(ClosedNotification { id, reason })
    }

    pub fn expire(&mut self, now: Instant) -> Vec<ClosedNotification> {
        let mut closed = Vec::new();
        self.notifications.retain(|notification| {
            if notification
                .expires_at
                .is_some_and(|expires_at| expires_at <= now)
            {
                closed.push(ClosedNotification {
                    id: notification.id,
                    reason: 1,
                });
                false
            } else {
                true
            }
        });
        closed
    }

    pub fn newest(&self) -> Option<DesktopNotification> {
        self.notifications.back().cloned()
    }

    pub fn has_action(&self, id: u32, key: &str) -> bool {
        self.notifications.iter().any(|notification| {
            notification.id == id && notification.actions.iter().any(|action| action.key == key)
        })
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.notifications.len()
    }

    fn allocate_id(&mut self) -> u32 {
        loop {
            let id = self.next_id.max(1);
            self.next_id = id.wrapping_add(1).max(1);
            if !self
                .notifications
                .iter()
                .any(|notification| notification.id == id)
            {
                return id;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        MAX_NOTIFICATION_ACTIONS, NotificationAction, NotificationRequest, NotificationStore,
    };
    use std::time::{Duration, Instant};

    fn notify(store: &mut NotificationStore, replaces_id: u32, now: Instant) -> u32 {
        store
            .notify(
                replaces_id,
                NotificationRequest {
                    app_name: "Test App".into(),
                    summary: "Summary".into(),
                    body: "Body".into(),
                    actions: Vec::new(),
                    expire_timeout_ms: 1_000,
                },
                now,
            )
            .0
    }

    #[test]
    fn allocates_nonzero_ids_and_replaces_in_place() {
        let now = Instant::now();
        let mut store = NotificationStore::default();
        let id = notify(&mut store, 0, now);
        let replacement = notify(&mut store, id, now);
        assert_ne!(id, 0);
        assert_eq!(replacement, id);
        assert_eq!(store.len(), 1);
    }

    #[test]
    fn explicit_close_uses_requested_reason() {
        let now = Instant::now();
        let mut store = NotificationStore::default();
        let id = notify(&mut store, 0, now);
        assert_eq!(store.close(id, 3).unwrap().reason, 3);
        assert!(store.newest().is_none());
    }

    #[test]
    fn expires_elapsed_notifications() {
        let now = Instant::now();
        let mut store = NotificationStore::default();
        let id = notify(&mut store, 0, now);
        assert!(store.expire(now + Duration::from_millis(999)).is_empty());
        assert_eq!(store.expire(now + Duration::from_secs(1))[0].id, id);
    }

    #[test]
    fn zero_timeout_persists_until_closed() {
        let now = Instant::now();
        let mut store = NotificationStore::default();
        let id = store
            .notify(
                0,
                NotificationRequest {
                    app_name: "Persistent App".into(),
                    summary: "Persistent".into(),
                    body: String::new(),
                    actions: Vec::new(),
                    expire_timeout_ms: 0,
                },
                now,
            )
            .0;
        assert!(store.expire(now + Duration::from_secs(86_400)).is_empty());
        assert_eq!(store.close(id, 3).unwrap().id, id);
    }

    #[test]
    fn bounds_notification_history() {
        let now = Instant::now();
        let mut store = NotificationStore::default();
        let first = notify(&mut store, 0, now);
        let mut discarded = None;
        for _ in 0..100 {
            discarded = store
                .notify(
                    0,
                    NotificationRequest {
                        app_name: "App".into(),
                        summary: "New".into(),
                        body: String::new(),
                        actions: Vec::new(),
                        expire_timeout_ms: 1_000,
                    },
                    now,
                )
                .1;
        }
        assert_eq!(store.len(), 100);
        assert_eq!(discarded.unwrap().id, first);
    }

    #[test]
    fn replacement_updates_bounded_action_identity_in_place() {
        let now = Instant::now();
        let mut store = NotificationStore::default();
        let actions = (0..MAX_NOTIFICATION_ACTIONS)
            .map(|index| NotificationAction {
                key: format!("key-{index}"),
                label: format!("Action {index}"),
            })
            .collect::<Vec<_>>();
        let id = store
            .notify(
                0,
                NotificationRequest {
                    app_name: "App".into(),
                    summary: "First".into(),
                    body: String::new(),
                    actions: actions.clone(),
                    expire_timeout_ms: 0,
                },
                now,
            )
            .0;
        assert!(store.has_action(id, "key-2"));

        store.notify(
            id,
            NotificationRequest {
                app_name: "App".into(),
                summary: "Replacement".into(),
                body: String::new(),
                actions: vec![NotificationAction {
                    key: "replacement".into(),
                    label: "Replace".into(),
                }],
                expire_timeout_ms: 0,
            },
            now,
        );
        assert!(!store.has_action(id, "key-2"));
        assert!(store.has_action(id, "replacement"));
        assert_eq!(store.newest().unwrap().actions.len(), 1);
    }
}
