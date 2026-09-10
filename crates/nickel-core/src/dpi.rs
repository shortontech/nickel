//! Exact per-output scale and cross-output surface selection policy.

use std::{
    collections::{BTreeMap, BTreeSet},
    fs, io,
    path::Path,
};

pub use crate::geometry::LogicalRect;
use nickel_storage::{atomic_write, config_path};

/// Wayland fractional-scale units. 120 units are exactly one logical-to-physical pixel ratio.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct Scale120(u32);

impl Scale120 {
    pub const ONE: Self = Self(120);

    pub const fn new(units: u32) -> Option<Self> {
        if units == 0 { None } else { Some(Self(units)) }
    }

    pub const fn units(self) -> u32 {
        self.0
    }

    pub fn factor(self) -> f64 {
        f64::from(self.0) / 120.0
    }

    pub fn physical_extent(self, logical: u32) -> u32 {
        let scaled = u64::from(logical) * u64::from(self.0);
        u32::try_from(scaled.div_ceil(120)).unwrap_or(u32::MAX)
    }

    pub const fn integer_buffer_scale(self) -> i32 {
        self.0.div_ceil(120) as i32
    }
}

impl Default for Scale120 {
    fn default() -> Self {
        Self::ONE
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OutputScale {
    pub identity: String,
    pub geometry: LogicalRect,
    pub scale: Scale120,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SurfaceScaleState {
    pub effective_output: Option<String>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct EffectiveOutput {
    pub output: Option<String>,
    pub entered: BTreeSet<String>,
}

/// Selects one rendering scale while retaining every intersected output for enter/leave events.
/// `hysteresis_area` is a logical-pixel area advantage required to displace the prior output.
pub fn select_effective_output(
    surface: LogicalRect,
    outputs: &[OutputScale],
    previous: Option<&str>,
    active: Option<&str>,
    hysteresis_area: u64,
) -> EffectiveOutput {
    let mut intersections = outputs
        .iter()
        .filter_map(|output| {
            let area = surface.intersection_area(output.geometry);
            (area > 0).then_some((output, area))
        })
        .collect::<Vec<_>>();
    let entered = intersections
        .iter()
        .map(|(output, _)| output.identity.clone())
        .collect();
    if intersections.is_empty() {
        return EffectiveOutput {
            output: None,
            entered,
        };
    }
    intersections.sort_by(|(left, left_area), (right, right_area)| {
        right_area
            .cmp(left_area)
            .then_with(|| {
                (Some(left.identity.as_str()) != previous)
                    .cmp(&(Some(right.identity.as_str()) != previous))
            })
            .then_with(|| {
                (Some(left.identity.as_str()) != active)
                    .cmp(&(Some(right.identity.as_str()) != active))
            })
            .then_with(|| left.identity.cmp(&right.identity))
    });
    let mut winner = intersections[0];
    if let Some(previous) = previous
        && let Some(prior) = intersections
            .iter()
            .copied()
            .find(|(output, _)| output.identity == previous)
        && winner.1.saturating_sub(prior.1) <= hysteresis_area
    {
        winner = prior;
    }
    EffectiveOutput {
        output: Some(winner.0.identity.clone()),
        entered,
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PersistedOutputScales {
    scales: BTreeMap<String, Scale120>,
}

impl PersistedOutputScales {
    pub fn get(&self, identity: &str) -> Option<Scale120> {
        self.scales.get(identity).copied()
    }
    pub fn set(&mut self, identity: impl Into<String>, scale: Scale120) {
        self.scales.insert(identity.into(), scale);
    }
    pub fn remove(&mut self, identity: &str) {
        self.scales.remove(identity);
    }
    pub fn parse(text: &str) -> Self {
        let mut value = Self::default();
        for line in text.lines() {
            let Some((identity, units)) = line.split_once('\t') else {
                continue;
            };
            let Some(scale) = units.parse().ok().and_then(Scale120::new) else {
                continue;
            };
            if !identity.is_empty() {
                value.set(identity, scale);
            }
        }
        value
    }
    pub fn serialize(&self) -> String {
        self.scales
            .iter()
            .map(|(id, scale)| format!("{id}\t{}\n", scale.units()))
            .collect()
    }
    pub fn load(path: &Path) -> io::Result<Self> {
        match fs::read_to_string(path) {
            Ok(text) => Ok(Self::parse(&text)),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Self::default()),
            Err(error) => Err(error),
        }
    }
    pub fn save(&self, path: &Path) -> io::Result<()> {
        atomic_write(path, self.serialize())
    }
    pub fn load_default() -> io::Result<Self> {
        Self::load(&config_path("display-scales.tsv")?)
    }
    pub fn save_default(&self) -> io::Result<()> {
        self.save(&config_path("display-scales.tsv")?)
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ApplicationScalePolicy {
    #[default]
    FollowNickel,
    Unchanged,
    Custom(Scale120),
}

/// A write-ahead record. It is resolved only by a later authorized read-back;
/// interrupted external setters are never assumed to have failed or succeeded.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ToolkitScaleIntent {
    pub previous: String,
    pub requested: String,
    pub restoring: bool,
    /// True only after this setter returned native terminal completion evidence.
    pub terminal: bool,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ApplicationScaleSettings {
    pub policy: ApplicationScalePolicy,
    /// Values are recorded only after Nickel successfully owns a toolkit write.
    pub owned_gtk_previous: Option<String>,
    pub owned_gtk_applied: Option<String>,
    pub owned_qt_previous: Option<String>,
    pub owned_qt_applied: Option<String>,
    pub pending_gtk: Option<ToolkitScaleIntent>,
    pub pending_qt: Option<ToolkitScaleIntent>,
}

const MAX_APPLICATION_SCALE_BYTES: usize = 64 * 1024;

impl ApplicationScaleSettings {
    pub fn launch_environment(&self, linux: bool) -> BTreeMap<String, String> {
        if !linux {
            return BTreeMap::new();
        }
        match self.policy {
            // Native Wayland clients receive compositor scale. Clearing common
            // overrides prevents accidental environment + compositor stacking.
            ApplicationScalePolicy::FollowNickel => BTreeMap::from([
                ("GDK_SCALE".into(), String::new()),
                ("GDK_DPI_SCALE".into(), String::new()),
                ("QT_SCALE_FACTOR".into(), String::new()),
            ]),
            ApplicationScalePolicy::Unchanged => BTreeMap::new(),
            ApplicationScalePolicy::Custom(scale) => BTreeMap::from([
                ("GDK_SCALE".into(), scale.integer_buffer_scale().to_string()),
                (
                    "GDK_DPI_SCALE".into(),
                    format!(
                        "{:.6}",
                        scale.factor() / f64::from(scale.integer_buffer_scale())
                    ),
                ),
                ("QT_SCALE_FACTOR".into(), format!("{:.6}", scale.factor())),
            ]),
        }
    }

    pub fn load(path: &Path) -> io::Result<Self> {
        let Some(bytes) = nickel_storage::read_regular_file(path, MAX_APPLICATION_SCALE_BYTES)?
        else {
            return Ok(Self::default());
        };
        let text = String::from_utf8(bytes).map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid application scale settings",
            )
        })?;
        let mut settings = Self::default();
        for line in text.lines() {
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            match key {
                "policy" if value == "follow" => {
                    settings.policy = ApplicationScalePolicy::FollowNickel
                }
                "policy" if value == "unchanged" => {
                    settings.policy = ApplicationScalePolicy::Unchanged
                }
                "policy" if value.starts_with("custom:") => {
                    if let Some(scale) = value[7..].parse().ok().and_then(Scale120::new) {
                        settings.policy = ApplicationScalePolicy::Custom(scale);
                    }
                }
                "gtk_previous" => settings.owned_gtk_previous = Some(value.into()),
                "gtk_applied" => settings.owned_gtk_applied = Some(value.into()),
                "qt_previous" => settings.owned_qt_previous = Some(value.into()),
                "qt_applied" => settings.owned_qt_applied = Some(value.into()),
                "gtk_pending" | "qt_pending" => {
                    let mut values = value.split('|');
                    if let (Some(restore), Some(previous), Some(requested), terminal, None) = (
                        values.next(),
                        values.next(),
                        values.next(),
                        values.next(),
                        values.next(),
                    ) {
                        if matches!(restore, "0" | "1")
                            && previous.len() <= 64
                            && requested.len() <= 64
                            && matches!(terminal, None | Some("0") | Some("1"))
                        {
                            let intent = Some(ToolkitScaleIntent {
                                previous: previous.into(),
                                requested: requested.into(),
                                restoring: restore == "1",
                                terminal: terminal == Some("1"),
                            });
                            if key == "gtk_pending" {
                                settings.pending_gtk = intent;
                            } else {
                                settings.pending_qt = intent;
                            }
                        } else {
                            return Err(io::Error::new(
                                io::ErrorKind::InvalidData,
                                "invalid toolkit intent",
                            ));
                        }
                    } else {
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidData,
                            "invalid toolkit intent",
                        ));
                    }
                }
                _ => {}
            }
        }
        Ok(settings)
    }
    pub fn save(&self, path: &Path) -> io::Result<()> {
        let _lock = nickel_storage::TransactionLock::try_acquire(path)?;
        #[cfg(target_os = "linux")]
        {
            self.stage(path)?.commit(|| Ok(()))?.sync()
        }
        #[cfg(not(target_os = "linux"))]
        {
            nickel_storage::atomic_write(path, self.encode()?)
        }
    }
    pub fn stage(&self, path: &Path) -> io::Result<nickel_storage::StagedDurableWrite> {
        nickel_storage::stage_durable_write(path, self.encode()?)
    }
    fn encode(&self) -> io::Result<String> {
        let policy = match self.policy {
            ApplicationScalePolicy::FollowNickel => "follow".into(),
            ApplicationScalePolicy::Unchanged => "unchanged".into(),
            ApplicationScalePolicy::Custom(scale) => format!("custom:{}", scale.units()),
        };
        let clean = |value: &str| value.replace(['\n', '\r'], " ");
        let mut text = format!("version=1\npolicy={policy}\n");
        if let Some(value) = &self.owned_gtk_previous {
            text.push_str(&format!("gtk_previous={}\n", clean(value)));
        }
        if let Some(value) = &self.owned_gtk_applied {
            text.push_str(&format!("gtk_applied={}\n", clean(value)));
        }
        if let Some(value) = &self.owned_qt_previous {
            text.push_str(&format!("qt_previous={}\n", clean(value)));
        }
        if let Some(value) = &self.owned_qt_applied {
            text.push_str(&format!("qt_applied={}\n", clean(value)));
        }
        for (name, intent) in [
            ("gtk_pending", &self.pending_gtk),
            ("qt_pending", &self.pending_qt),
        ] {
            if let Some(intent) = intent {
                if [&intent.previous, &intent.requested].iter().any(|value| {
                    value.len() > 64 || value.chars().any(|ch| ch.is_control() || ch == '|')
                }) {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "invalid toolkit intent",
                    ));
                }
                text.push_str(&format!(
                    "{name}={}|{}|{}|{}\n",
                    u8::from(intent.restoring),
                    intent.previous,
                    intent.requested,
                    u8::from(intent.terminal)
                ));
            }
        }
        if text.len() > MAX_APPLICATION_SCALE_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "application scale settings exceed size limit",
            ));
        }
        Ok(text)
    }
    pub fn load_default() -> io::Result<Self> {
        Self::load(&config_path("application-scale.conf")?)
    }
    pub fn save_default(&self) -> io::Result<()> {
        self.save(&config_path("application-scale.conf")?)
    }
}

/// Cross-process ownership spans the complete toolkit transaction, not one rename.
pub struct ApplicationScaleJournal {
    _lock: nickel_storage::TransactionLock,
    path: std::path::PathBuf,
    expected: Option<Vec<u8>>,
}
impl ApplicationScaleJournal {
    pub fn open_default() -> io::Result<Self> {
        Self::open(config_path("application-scale.conf")?)
    }
    pub fn open(path: std::path::PathBuf) -> io::Result<Self> {
        let lock = nickel_storage::TransactionLock::try_acquire(&path)?;
        let expected = Self::bytes(&path)?;
        Ok(Self {
            _lock: lock,
            path,
            expected,
        })
    }
    fn bytes(path: &Path) -> io::Result<Option<Vec<u8>>> {
        nickel_storage::read_regular_file(path, MAX_APPLICATION_SCALE_BYTES)
    }
    pub fn load(&self) -> io::Result<ApplicationScaleSettings> {
        let settings = ApplicationScaleSettings::load(&self.path)?;
        if Self::bytes(&self.path)? != self.expected {
            return Err(io::Error::other("application scale settings changed"));
        }
        Ok(settings)
    }
    pub fn persist(&mut self, settings: &ApplicationScaleSettings) -> io::Result<()> {
        settings
            .stage(&self.path)?
            .commit(|| {
                if Self::bytes(&self.path)? != self.expected {
                    return Err(io::Error::other("application scale settings changed"));
                }
                Ok(())
            })?
            .sync()?;
        self.expected = Some(settings.encode()?.into_bytes());
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn output(id: &str, x: i32, scale: u32) -> OutputScale {
        OutputScale {
            identity: id.into(),
            geometry: LogicalRect {
                x,
                y: -100,
                width: 1000,
                height: 800,
            },
            scale: Scale120::new(scale).unwrap(),
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn journal_serializes_independent_callers_and_rejects_unowned_replacement() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("scale");
        let mut first = ApplicationScaleJournal::open(path.clone()).unwrap();
        assert_eq!(
            ApplicationScaleJournal::open(path.clone())
                .err()
                .unwrap()
                .kind(),
            io::ErrorKind::WouldBlock
        );
        first.persist(&ApplicationScaleSettings::default()).unwrap();
        assert_eq!(
            ApplicationScaleSettings::default()
                .save(&path)
                .unwrap_err()
                .kind(),
            io::ErrorKind::WouldBlock
        );
        fs::write(&path, "policy=unchanged\n").unwrap();
        assert!(first.persist(&ApplicationScaleSettings::default()).is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), "policy=unchanged\n");
        drop(first);
        assert_eq!(
            ApplicationScaleJournal::open(path)
                .unwrap()
                .load()
                .unwrap()
                .policy,
            ApplicationScalePolicy::Unchanged
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn application_scale_intent_rejects_malformed_input_and_denied_commit() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("application-scale.conf");
        for invalid in [
            "gtk_pending=0|1",
            "qt_pending=2|1|2",
            "gtk_pending=0|1|2|3|4",
        ] {
            fs::write(&path, invalid).unwrap();
            assert!(ApplicationScaleSettings::load(&path).is_err());
        }
        let original = ApplicationScaleSettings::default();
        original.save(&path).unwrap();
        let before = fs::read(&path).unwrap();
        let mut changed = original;
        changed.pending_qt = Some(ToolkitScaleIntent {
            previous: "1".into(),
            requested: "2".into(),
            restoring: false,
            terminal: false,
        });
        assert!(
            changed
                .stage(&path)
                .unwrap()
                .commit(|| Err(io::Error::other("revoked")))
                .is_err()
        );
        assert_eq!(fs::read(&path).unwrap(), before);
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 2);
    }

    #[test]
    fn exact_scale_conversion_rounds_only_at_buffer_boundary() {
        assert_eq!(Scale120::new(150).unwrap().factor(), 1.25);
        assert_eq!(Scale120::new(150).unwrap().physical_extent(1919), 2399);
        assert_eq!(Scale120::new(180).unwrap().integer_buffer_scale(), 2);
        assert_eq!(Scale120::new(0), None);
    }

    #[test]
    fn largest_intersection_ties_and_hysteresis_are_stable() {
        let outputs = [output("left", -1000, 120), output("right", 0, 180)];
        let equal = LogicalRect {
            x: -500,
            y: 0,
            width: 1000,
            height: 600,
        };
        assert_eq!(
            select_effective_output(equal, &outputs, Some("left"), Some("right"), 0)
                .output
                .as_deref(),
            Some("left")
        );
        assert_eq!(
            select_effective_output(equal, &outputs, None, Some("right"), 0)
                .output
                .as_deref(),
            Some("right")
        );
        assert_eq!(
            select_effective_output(equal, &outputs, None, None, 0)
                .output
                .as_deref(),
            Some("left")
        );
        let barely_right = LogicalRect { x: -499, ..equal };
        assert_eq!(
            select_effective_output(barely_right, &outputs, Some("left"), None, 2_000)
                .output
                .as_deref(),
            Some("left")
        );
        let clearly_right = LogicalRect { x: -450, ..equal };
        assert_eq!(
            select_effective_output(clearly_right, &outputs, Some("left"), None, 2_000)
                .output
                .as_deref(),
            Some("right")
        );
        assert_eq!(
            select_effective_output(equal, &outputs, None, None, 0).entered,
            BTreeSet::from(["left".into(), "right".into()])
        );
    }

    #[test]
    fn repeated_boundary_crossing_does_not_oscillate() {
        let outputs = [output("a", 0, 120), output("b", 1000, 240)];
        let mut state = Some("a".to_owned());
        for x in [749, 751, 748, 752, 747, 753] {
            state = select_effective_output(
                LogicalRect {
                    x,
                    y: 0,
                    width: 500,
                    height: 500,
                },
                &outputs,
                state.as_deref(),
                None,
                5_000,
            )
            .output;
            assert_eq!(state.as_deref(), Some("a"));
        }
        state = select_effective_output(
            LogicalRect {
                x: 800,
                y: 0,
                width: 500,
                height: 500,
            },
            &outputs,
            state.as_deref(),
            None,
            5_000,
        )
        .output;
        assert_eq!(state.as_deref(), Some("b"));
    }

    #[test]
    fn top_level_and_child_transition_across_all_supported_scale_classes() {
        let outputs = [
            output("one", 0, 120),
            output("one-quarter", 1000, 150),
            output("one-half", 2000, 180),
            output("two", 3000, 240),
        ];
        let mut previous = None;
        for (x, identity, scale) in [
            (100, "one", 120),
            (1100, "one-quarter", 150),
            (2100, "one-half", 180),
            (3100, "two", 240),
        ] {
            let selected = select_effective_output(
                LogicalRect {
                    x,
                    y: 0,
                    width: 600,
                    height: 500,
                },
                &outputs,
                previous.as_deref(),
                Some(identity),
                4_096,
            );
            assert_eq!(selected.output.as_deref(), Some(identity));
            let effective = outputs
                .iter()
                .find(|output| selected.output.as_deref() == Some(output.identity.as_str()))
                .unwrap();
            assert_eq!(effective.scale.units(), scale);
            // Popups and subsurfaces inherit this selected identity instead of
            // independently evaluating their smaller geometry.
            let child_effective = selected.output.clone();
            assert_eq!(child_effective, selected.output);
            previous = selected.output;
        }

        let remaining = &outputs[..3];
        let after_hotplug = select_effective_output(
            LogicalRect {
                x: 3100,
                y: 0,
                width: 600,
                height: 500,
            },
            remaining,
            previous.as_deref(),
            Some("one-half"),
            4_096,
        );
        assert_eq!(
            after_hotplug.output, None,
            "stranded geometry is rescued before reselection"
        );
    }

    #[test]
    fn persistence_is_stable_and_ignores_corruption() {
        let parsed =
            PersistedOutputScales::parse("panel-edid\t150\nbad\t0\nmalformed\nwide-edid\t240\n");
        assert_eq!(parsed.get("panel-edid"), Scale120::new(150));
        assert_eq!(parsed.get("bad"), None);
        assert_eq!(PersistedOutputScales::parse(&parsed.serialize()), parsed);
    }

    #[test]
    fn scale_and_application_preferences_replace_files_atomically() {
        let directory = tempfile::tempdir().unwrap();
        let output_path = directory.path().join("outputs");
        let mut outputs = PersistedOutputScales::default();
        outputs.set("stable-edid", Scale120::new(180).unwrap());
        outputs.save(&output_path).unwrap();
        assert_eq!(PersistedOutputScales::load(&output_path).unwrap(), outputs);

        let app_path = directory.path().join("applications");
        let apps = ApplicationScaleSettings {
            policy: ApplicationScalePolicy::Custom(Scale120::new(150).unwrap()),
            owned_gtk_previous: Some("uint32 1".into()),
            owned_gtk_applied: Some("2".into()),
            owned_qt_previous: Some("1.0".into()),
            owned_qt_applied: Some("1.25".into()),
            pending_qt: Some(ToolkitScaleIntent {
                previous: "1.25".into(),
                requested: "1.5".into(),
                restoring: false,
                terminal: false,
            }),
            ..Default::default()
        };
        apps.save(&app_path).unwrap();
        assert_eq!(ApplicationScaleSettings::load(&app_path).unwrap(), apps);
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 3);
    }

    #[test]
    fn application_scale_read_and_save_reject_oversized_files_without_partial_policy() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("application-scale.conf");
        let mut boundary = String::from("policy=unchanged\n");
        boundary.extend(std::iter::repeat_n(
            'x',
            MAX_APPLICATION_SCALE_BYTES - boundary.len(),
        ));
        fs::write(&path, &boundary).unwrap();
        assert_eq!(
            ApplicationScaleSettings::load(&path).unwrap().policy,
            ApplicationScalePolicy::Unchanged
        );
        boundary.push('x');
        fs::write(&path, &boundary).unwrap();
        assert_eq!(
            ApplicationScaleSettings::load(&path).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
        let saved = ApplicationScaleSettings::default();
        saved.save(&path).unwrap();
        let original = fs::read(&path).unwrap();
        let oversized = ApplicationScaleSettings {
            owned_gtk_previous: Some("x".repeat(MAX_APPLICATION_SCALE_BYTES)),
            ..saved
        };
        assert_eq!(
            oversized.save(&path).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
        assert_eq!(fs::read(&path).unwrap(), original);
    }

    #[test]
    fn launch_environment_never_stacks_follow_mode_and_preserves_unchanged_mode() {
        let follow = ApplicationScaleSettings::default().launch_environment(true);
        assert_eq!(follow.get("QT_SCALE_FACTOR").map(String::as_str), Some(""));
        assert!(
            ApplicationScaleSettings {
                policy: ApplicationScalePolicy::Unchanged,
                ..Default::default()
            }
            .launch_environment(true)
            .is_empty()
        );
        let custom = ApplicationScaleSettings {
            policy: ApplicationScalePolicy::Custom(Scale120::new(150).unwrap()),
            ..Default::default()
        }
        .launch_environment(true);
        assert_eq!(custom["GDK_SCALE"], "2");
        assert_eq!(custom["QT_SCALE_FACTOR"], "1.250000");
    }
}
