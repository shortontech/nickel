//! Deterministic reporting shared by ignored release-profile admission workloads.
//!
//! Timing is necessarily host dependent, but workload metadata, exact native work
//! counts, and percentile selection must be stable so admission logs can be
//! compared without reverse engineering each benchmark's ad-hoc output.

use std::{collections::BTreeMap, time::Duration};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct DurationDistribution {
    pub p50: Duration,
    pub p95: Duration,
    pub p99: Duration,
    pub max: Duration,
}

impl DurationDistribution {
    pub(crate) fn from_samples(samples: &[Duration]) -> Self {
        assert!(
            !samples.is_empty(),
            "admission distributions require samples"
        );
        let mut sorted = samples.to_vec();
        sorted.sort_unstable();
        Self {
            p50: percentile(&sorted, 50),
            p95: percentile(&sorted, 95),
            p99: percentile(&sorted, 99),
            max: *sorted.last().expect("non-empty samples"),
        }
    }
}

fn percentile(sorted: &[Duration], percentile: usize) -> Duration {
    // Nearest-rank percentile with a zero-based index. This selects the same
    // member on every host and, unlike interpolation, never invents a duration.
    let index = sorted.len().saturating_mul(percentile).div_ceil(100);
    sorted[index.saturating_sub(1).min(sorted.len() - 1)]
}

#[derive(Debug)]
pub(crate) struct AdmissionReport<'a> {
    suite: &'a str,
    workload: &'a str,
    metadata: BTreeMap<&'a str, u64>,
    work: BTreeMap<&'a str, u64>,
    timings: BTreeMap<&'a str, DurationDistribution>,
}

impl<'a> AdmissionReport<'a> {
    pub(crate) fn new(suite: &'a str, workload: &'a str) -> Self {
        Self {
            suite,
            workload,
            metadata: BTreeMap::new(),
            work: BTreeMap::new(),
            timings: BTreeMap::new(),
        }
    }

    pub(crate) fn metadata(mut self, name: &'a str, value: usize) -> Self {
        self.metadata.insert(name, value as u64);
        self
    }

    pub(crate) fn work(mut self, name: &'a str, value: usize) -> Self {
        self.work.insert(name, value as u64);
        self
    }

    pub(crate) fn timings(mut self, name: &'a str, samples: &[Duration]) -> Self {
        self.timings
            .insert(name, DurationDistribution::from_samples(samples));
        self
    }

    #[allow(dead_code)]
    pub(crate) fn emit(&self) {
        eprintln!("{}", self.render());
    }

    fn render(&self) -> String {
        let metadata = render_u64_map(&self.metadata);
        let work = render_u64_map(&self.work);
        let timings = self
            .timings
            .iter()
            .map(|(name, distribution)| {
                format!(
                    "\"{name}\":{{\"p50_ns\":{},\"p95_ns\":{},\"p99_ns\":{},\"max_ns\":{}}}",
                    distribution.p50.as_nanos(),
                    distribution.p95.as_nanos(),
                    distribution.p99.as_nanos(),
                    distribution.max.as_nanos()
                )
            })
            .collect::<Vec<_>>()
            .join(",");
        format!(
            "nickel_release_admission={{\"schema\":1,\"suite\":\"{}\",\"workload\":\"{}\",\"metadata\":{{{metadata}}},\"work\":{{{work}}},\"timings\":{{{timings}}}}}",
            self.suite, self.workload
        )
    }
}

fn render_u64_map(values: &BTreeMap<&str, u64>) -> String {
    values
        .iter()
        .map(|(name, value)| format!("\"{name}\":{value}"))
        .collect::<Vec<_>>()
        .join(",")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn distributions_use_reproducible_nearest_rank_members() {
        let samples = (1..=100).map(Duration::from_nanos).collect::<Vec<_>>();
        assert_eq!(
            DurationDistribution::from_samples(&samples),
            DurationDistribution {
                p50: Duration::from_nanos(50),
                p95: Duration::from_nanos(95),
                p99: Duration::from_nanos(99),
                max: Duration::from_nanos(100),
            }
        );
    }

    #[test]
    fn report_is_stable_and_machine_readable() {
        let report = AdmissionReport::new("retained", "unchanged")
            .metadata("nodes", 2_000)
            .metadata("iterations", 3)
            .work("nodes_measured", 0)
            .timings(
                "retained",
                &[Duration::from_nanos(7), Duration::from_nanos(3)],
            );
        assert_eq!(
            report.render(),
            "nickel_release_admission={\"schema\":1,\"suite\":\"retained\",\"workload\":\"unchanged\",\"metadata\":{\"iterations\":3,\"nodes\":2000},\"work\":{\"nodes_measured\":0},\"timings\":{\"retained\":{\"p50_ns\":3,\"p95_ns\":7,\"p99_ns\":7,\"max_ns\":7}}}"
        );
    }
}
