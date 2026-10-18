//! Owner-only application membership from normalized catalog launch receipts.
//! Shared executables are never sufficient: every unpackaged process must have
//! its own owner-attested launch, exact process incarnation and retained image.
use std::collections::{BTreeMap, BTreeSet};

const MAX_ENTRIES: usize = 4096;
const MAX_RECEIPTS: usize = 1024;

const MAX_CATALOG_AGE: std::time::Duration = std::time::Duration::from_secs(15);

/// Native FILETIME bounds captured immediately around the owner's invocation.
/// Retained process identity alone does not establish that the call created it.
#[derive(Clone, Copy)]
struct LaunchInterval {
    started: u64,
    completed: u64,
}
impl LaunchInterval {
    fn admits(self, (pid, created): (u32, u64)) -> bool {
        pid != 0
            && self.started != 0
            && self.completed >= self.started
            && created > self.started
            && created <= self.completed
    }
}

fn fresh_catalog(
    started: std::time::Instant,
    completed: std::time::Instant,
    now: std::time::Instant,
) -> bool {
    completed.checked_duration_since(started).is_some()
        && now.checked_duration_since(completed).is_some()
        && now
            .checked_duration_since(started)
            .is_some_and(|age| age < MAX_CATALOG_AGE)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RuntimePolicy {
    /// The program may be an interpreter. Authorize only exact owner launches.
    LaunchBound,
    /// URL, delegated activation, unresolved target or unavailable descriptor.
    Unavailable,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct Descriptor {
    id: String,
    identity: String,
    digest: [u8; 32],
    policy: RuntimePolicy,
}

trait ProcessEvidence {
    type Image: Clone;
    fn incarnation(&self) -> (u32, u64);
    fn is_live(&self) -> bool;
    fn image(&self) -> Option<Self::Image>;
    fn same_image(left: &Self::Image, right: &Self::Image) -> bool;
}

struct Entry<E> {
    descriptor: Descriptor,
    generation: u64,
    image: Option<E>,
}
struct Receipt<P> {
    catalog_id: String,
    generation: u64,
    process: P,
}
struct LeasePin<E> {
    catalog_id: String,
    generation: u64,
    // Independent of process inventory: retained after the final process exits.
    _image: E,
}
struct Registry<P: ProcessEvidence> {
    generation: u64,
    entries: BTreeMap<String, Entry<P::Image>>,
    receipts: BTreeMap<(u32, u64), Receipt<P>>,
    leases: BTreeMap<u64, LeasePin<P::Image>>,
}
impl<P: ProcessEvidence> Default for Registry<P> {
    fn default() -> Self {
        Self {
            generation: 0,
            entries: BTreeMap::new(),
            receipts: BTreeMap::new(),
            leases: BTreeMap::new(),
        }
    }
}
impl<P: ProcessEvidence> Registry<P> {
    fn tracked_children(&self) -> usize {
        self.receipts
            .values()
            .filter(|receipt| receipt.process.is_live())
            .count()
    }

    fn retire(&mut self, id: &str, revoke: &mut impl FnMut(u64)) {
        let leases: Vec<_> = self
            .leases
            .iter()
            .filter_map(|(lease, pin)| (pin.catalog_id == id).then_some(*lease))
            .collect();
        // Revoke/cancel real authority before releasing any lease or catalog pin.
        for lease in leases {
            revoke(lease);
            self.leases.remove(&lease);
        }
        self.receipts.retain(|_, receipt| receipt.catalog_id != id);
        self.entries.remove(id);
    }

    fn reconcile(&mut self, descriptors: Vec<Descriptor>, mut revoke: impl FnMut(u64)) {
        let mut next = BTreeMap::new();
        let mut duplicate = BTreeSet::new();
        if descriptors.len() <= MAX_ENTRIES {
            for descriptor in descriptors {
                if descriptor.id.is_empty()
                    || descriptor.id.len() > 4096
                    || descriptor.identity.len() > 128
                {
                    continue;
                }
                let id = descriptor.id.clone();
                if next.insert(id.clone(), descriptor).is_some() {
                    duplicate.insert(id);
                }
            }
        }
        for id in duplicate {
            next.remove(&id);
        }
        let changed: Vec<_> = self
            .entries
            .iter()
            .filter_map(|(id, entry)| {
                (next.get(id) != Some(&entry.descriptor)).then_some(id.clone())
            })
            .collect();
        for id in changed {
            self.retire(&id, &mut revoke);
        }
        for (id, descriptor) in next {
            if self.entries.contains_key(&id) {
                continue;
            }
            let Some(generation) = self.generation.checked_add(1) else {
                continue;
            };
            self.generation = generation;
            self.entries.insert(
                id,
                Entry {
                    descriptor,
                    generation,
                    image: None,
                },
            );
        }
    }

    fn attest(
        &mut self,
        descriptor: &Descriptor,
        process: P,
        invocation: LaunchInterval,
        mut revoke: impl FnMut(u64),
    ) -> bool {
        if descriptor.policy != RuntimePolicy::LaunchBound
            || !invocation.admits(process.incarnation())
            || !process.is_live()
            || self.receipts.len() >= MAX_RECEIPTS
        {
            return false;
        }
        let Some(image) = process.image() else {
            return false;
        };
        let Some(entry) = self.entries.get(&descriptor.id) else {
            return false;
        };
        if &entry.descriptor != descriptor {
            return false;
        }
        // A changed executable cannot silently become the same application. Keep
        // the existing pin until all affected authority has been cancelled.
        if entry
            .image
            .as_ref()
            .is_some_and(|old| !P::same_image(old, &image))
        {
            self.retire(&descriptor.id, &mut revoke);
            let Some(generation) = self.generation.checked_add(1) else {
                return false;
            };
            self.generation = generation;
            self.entries.insert(
                descriptor.id.clone(),
                Entry {
                    descriptor: descriptor.clone(),
                    generation,
                    image: None,
                },
            );
        }
        let entry = self.entries.get_mut(&descriptor.id).unwrap();
        let incarnation = process.incarnation();
        // One exact process cannot be reassigned to another catalog application.
        if let Some(previous) = self.receipts.get(&incarnation) {
            return previous.catalog_id == descriptor.id && previous.generation == entry.generation;
        }
        entry.image = Some(image);
        self.receipts.insert(
            incarnation,
            Receipt {
                catalog_id: descriptor.id.clone(),
                generation: entry.generation,
                process,
            },
        );
        true
    }

    fn membership(&self, process: &P) -> Option<&str> {
        if !process.is_live() {
            return None;
        }
        let receipt = self.receipts.get(&process.incarnation())?;
        if !receipt.process.is_live() {
            return None;
        }
        let entry = self.entries.get(&receipt.catalog_id)?;
        if entry.generation != receipt.generation {
            return None;
        }
        let image = process.image()?;
        if !P::same_image(entry.image.as_ref()?, &image) {
            return None;
        }
        Some(&entry.descriptor.identity)
    }

    fn sync_leases(&mut self, active: &[(u64, String)], mut revoke: impl FnMut(u64)) {
        let active_ids: BTreeSet<_> = active.iter().map(|(id, _)| *id).collect();
        self.leases.retain(|id, _| active_ids.contains(id));
        for (id, identity) in active {
            let entry = self.entries.iter().find(|(_, entry)| {
                &entry.descriptor.identity == identity
                    && entry.descriptor.policy == RuntimePolicy::LaunchBound
            });
            let Some((catalog_id, entry)) = entry else {
                revoke(*id);
                self.leases.remove(id);
                continue;
            };
            let Some(image) = entry.image.as_ref() else {
                revoke(*id);
                self.leases.remove(id);
                continue;
            };
            if let Some(pin) = self.leases.get(id) {
                if pin.catalog_id != *catalog_id || pin.generation != entry.generation {
                    revoke(*id);
                    self.leases.remove(id);
                }
                continue;
            }
            self.leases.insert(
                *id,
                LeasePin {
                    catalog_id: catalog_id.clone(),
                    generation: entry.generation,
                    _image: image.clone(),
                },
            );
        }
        // Process exit drops only the process receipt. Entry and active lease pins
        // remain; a future owner-attested launch still needs the same pinned image.
        self.receipts.retain(|_, receipt| receipt.process.is_live());
        // Do not keep installed executables locked forever when neither a live
        // receipt nor a lease needs them. Retiring the final pin also retires the
        // entry incarnation, so later file-ID reuse is a new evidence generation.
        let retained: BTreeSet<_> = self
            .receipts
            .values()
            .map(|receipt| receipt.catalog_id.as_str())
            .chain(self.leases.values().map(|lease| lease.catalog_id.as_str()))
            .collect();
        for (id, entry) in &mut self.entries {
            if entry.image.is_some() && !retained.contains(id.as_str()) {
                entry.image = None;
                if let Some(generation) = self.generation.checked_add(1) {
                    self.generation = generation;
                    entry.generation = generation;
                } else {
                    entry.descriptor.policy = RuntimePolicy::Unavailable;
                }
            }
        }
    }
}

#[cfg(target_os = "windows")]
pub(crate) mod native;

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    };
    struct Image {
        id: u64,
        drops: Arc<AtomicUsize>,
    }
    impl Drop for Image {
        fn drop(&mut self) {
            self.drops.fetch_add(1, Ordering::SeqCst);
        }
    }
    #[derive(Clone)]
    struct Process {
        key: (u32, u64),
        live: Arc<AtomicBool>,
        image: Arc<Image>,
    }
    impl ProcessEvidence for Process {
        type Image = Arc<Image>;
        fn incarnation(&self) -> (u32, u64) {
            self.key
        }
        fn is_live(&self) -> bool {
            self.live.load(Ordering::SeqCst)
        }
        fn image(&self) -> Option<Self::Image> {
            Some(self.image.clone())
        }
        fn same_image(left: &Self::Image, right: &Self::Image) -> bool {
            left.id == right.id
        }
    }
    fn process(pid: u32, created: u64, image: u64) -> Process {
        Process {
            key: (pid, created + 10),
            live: Arc::new(AtomicBool::new(true)),
            image: Arc::new(Image {
                id: image,
                drops: Arc::new(AtomicUsize::new(0)),
            }),
        }
    }
    fn descriptor(id: &str) -> Descriptor {
        Descriptor {
            id: id.into(),
            identity: format!("windows:catalog:{id}"),
            digest: [1; 32],
            policy: RuntimePolicy::LaunchBound,
        }
    }

    fn launch_interval() -> LaunchInterval {
        LaunchInterval {
            started: 10,
            completed: 1000,
        }
    }

    #[test]
    fn existing_process_and_wrong_incarnation_cannot_gain_a_launch_receipt() {
        let mut registry = Registry::default();
        let app = descriptor("app");
        registry.reconcile(vec![app.clone()], |_| {});
        let existing = process(7, 1, 9);
        let invocation = LaunchInterval {
            started: 12,
            completed: 20,
        };
        assert!(!registry.attest(&app, existing.clone(), invocation, |_| {}));
        assert_eq!(registry.membership(&existing), None);
        let newly_created = process(7, 3, 9);
        assert!(registry.attest(&app, newly_created.clone(), invocation, |_| {}));
        assert_eq!(registry.membership(&existing), None);
        assert_eq!(
            registry.membership(&newly_created),
            Some(app.identity.as_str())
        );
        for bounds in [
            LaunchInterval {
                started: 13,
                completed: 20,
            },
            LaunchInterval {
                started: 12,
                completed: 12,
            },
            LaunchInterval {
                started: 20,
                completed: 12,
            },
            LaunchInterval {
                started: 0,
                completed: 20,
            },
        ] {
            assert!(!bounds.admits(newly_created.incarnation()));
        }
        assert!(!invocation.admits((0, 13)));
        assert!(!invocation.admits((7, 21)));
    }

    #[test]
    fn delayed_and_hung_catalog_scans_never_become_fresh_at_delivery() {
        use std::time::{Duration, Instant};
        let start = Instant::now();
        let finish = start + Duration::from_secs(1);
        assert!(fresh_catalog(
            start,
            finish,
            finish + Duration::from_secs(1)
        ));
        assert!(!fresh_catalog(start, finish, start + MAX_CATALOG_AGE));
        assert!(!fresh_catalog(
            start,
            start + MAX_CATALOG_AGE,
            start + MAX_CATALOG_AGE
        ));
        assert!(!fresh_catalog(finish, start, finish));
        assert!(!fresh_catalog(start, finish, start));
    }

    #[test]
    fn shared_runtime_requires_each_exact_owner_launch_and_rejects_pid_reuse() {
        let mut registry = Registry::default();
        let app = descriptor("script-a");
        registry.reconcile(vec![app.clone()], |_| panic!("unexpected revoke"));
        let owned = process(1, 1, 7);
        assert!(registry.attest(&app, owned.clone(), launch_interval(), |_| {}));
        assert_eq!(
            registry.membership(&owned),
            Some("windows:catalog:script-a")
        );
        assert_eq!(registry.membership(&process(2, 1, 7)), None);
        assert_eq!(registry.membership(&process(1, 2, 7)), None);
        assert_eq!(registry.membership(&process(1, 1, 8)), None);
        assert_eq!(registry.tracked_children(), 1);
        owned.live.store(false, Ordering::SeqCst);
        assert_eq!(registry.tracked_children(), 0);
        assert_eq!(registry.membership(&owned), None);
    }

    #[test]
    fn last_process_exit_preserves_application_lease_pin_until_revocation() {
        let mut registry = Registry::default();
        let app = descriptor("app");
        registry.reconcile(vec![app.clone()], |_| {});
        let owned = process(1, 1, 7);
        let drops = owned.image.drops.clone();
        assert!(registry.attest(&app, owned.clone(), launch_interval(), |_| {}));
        registry.sync_leases(&[(10, app.identity.clone())], |_| {
            panic!("unexpected revoke")
        });
        owned.live.store(false, Ordering::SeqCst);
        drop(owned);
        registry.sync_leases(&[(10, app.identity.clone())], |_| {});
        assert!(registry.receipts.is_empty());
        assert_eq!(drops.load(Ordering::SeqCst), 0);
        let mut revoked = Vec::new();
        registry.reconcile(Vec::new(), |id| {
            assert_eq!(drops.load(Ordering::SeqCst), 0);
            revoked.push(id);
        });
        assert_eq!(revoked, vec![10]);
        assert_eq!(drops.load(Ordering::SeqCst), 1);
        registry.reconcile(vec![app.clone()], |_| {});
        registry.sync_leases(&[(10, app.identity)], |id| revoked.push(id));
        assert_eq!(revoked, vec![10, 10]);
    }

    #[test]
    fn final_lease_expiry_releases_file_pin_and_retires_its_generation() {
        let mut registry = Registry::default();
        let app = descriptor("app");
        registry.reconcile(vec![app.clone()], |_| {});
        let owned = process(1, 1, 7);
        let drops = owned.image.drops.clone();
        registry.attest(&app, owned.clone(), launch_interval(), |_| {});
        registry.sync_leases(&[(3, app.identity.clone())], |_| {});
        let generation = registry.entries["app"].generation;
        owned.live.store(false, Ordering::SeqCst);
        drop(owned);
        registry.sync_leases(&[], |_| {});
        assert_eq!(drops.load(Ordering::SeqCst), 1);
        assert!(registry.entries["app"].generation > generation);
        assert_eq!(registry.membership(&process(1, 1, 7)), None);
        let replacement = process(2, 1, 7);
        assert_eq!(registry.membership(&replacement), None);
        assert!(registry.attest(&app, replacement.clone(), launch_interval(), |_| {}));
        assert_eq!(
            registry.membership(&replacement),
            Some(app.identity.as_str())
        );
    }

    #[test]
    fn descriptor_reclassification_and_image_replacement_revoke_before_rebinding() {
        for reclassify in [false, true] {
            let mut registry = Registry::default();
            let mut app = descriptor("app");
            registry.reconcile(vec![app.clone()], |_| {});
            let original = process(1, 1, 7);
            registry.attest(&app, original.clone(), launch_interval(), |_| {});
            registry.sync_leases(&[(5, app.identity.clone())], |_| {});
            let old_generation = registry.entries["app"].generation;
            let mut revoked = Vec::new();
            if reclassify {
                app.policy = RuntimePolicy::Unavailable;
                registry.reconcile(vec![app.clone()], |id| revoked.push(id));
                assert!(!registry.attest(&app, process(2, 1, 8), launch_interval(), |_| {}));
            } else {
                assert!(
                    registry.attest(&app, process(2, 1, 8), launch_interval(), |id| revoked
                        .push(id))
                );
            }
            assert_eq!(revoked, vec![5]);
            assert!(registry.entries["app"].generation > old_generation);
            assert_eq!(registry.membership(&original), None);
        }
    }

    #[test]
    fn stale_and_duplicate_catalog_descriptors_do_not_admit_receipts() {
        let mut registry = Registry::default();
        let app = descriptor("app");
        registry.reconcile(vec![app.clone(), app.clone()], |_| {});
        assert!(!registry.attest(&app, process(1, 1, 7), launch_interval(), |_| {}));
        registry.reconcile(vec![app.clone()], |_| {});
        let mut stale = app.clone();
        stale.digest = [2; 32];
        assert!(!registry.attest(&stale, process(1, 1, 7), launch_interval(), |_| {}));
        registry.generation = u64::MAX;
        registry.reconcile(Vec::new(), |_| {});
        registry.reconcile(vec![app.clone()], |_| {});
        assert!(!registry.attest(&app, process(1, 1, 7), launch_interval(), |_| {}));
    }
}
