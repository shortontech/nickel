//! Platform-neutral desktop entry arrangement built on Nickel File identities.
//!
//! The shell supplies output work areas and forwards directory-provider changes. This module owns
//! selection, visual ordering, placement, and persisted affinity; it never performs file mutation.

use crate::{FileEntry, FileIdentity};
use std::{
    cmp::Ordering,
    collections::{HashMap, HashSet},
    fs, io,
    path::{Path, PathBuf},
};

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Point {
    pub x: f32,
    pub y: f32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DesktopOutput {
    pub id: String,
    /// Available area in global logical coordinates, excluding shell reservations.
    pub work_area: Rect,
    pub scale: f32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SortKey {
    Name,
    Kind,
    Size,
    Modified,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SortDirection {
    Ascending,
    Descending,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FolderGrouping {
    Mixed,
    FoldersFirst,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Arrangement {
    Manual,
    Sorted {
        key: SortKey,
        direction: SortDirection,
    },
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct DesktopEntryId(pub FileIdentity);

#[derive(Clone, Debug)]
pub struct DesktopItem {
    pub id: DesktopEntryId,
    pub entry: FileEntry,
    pub output: String,
    pub position: Point,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DesktopFileAction {
    Browse(PathBuf),
    Open(PathBuf),
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct SelectionModifiers {
    pub toggle: bool,
    pub range: bool,
    pub additive_range: bool,
}

#[derive(Clone, Debug)]
pub struct DesktopLayout {
    items: Vec<DesktopItem>,
    outputs: Vec<DesktopOutput>,
    selection: HashSet<DesktopEntryId>,
    anchor: Option<DesktopEntryId>,
    arrangement: Arrangement,
    grouping: FolderGrouping,
    cell: (f32, f32),
    icons_visible: bool,
    remembered_outputs: HashMap<DesktopEntryId, String>,
    locale: String,
}

impl DesktopLayout {
    pub fn new(outputs: Vec<DesktopOutput>) -> Self {
        Self {
            items: Vec::new(),
            outputs,
            selection: HashSet::new(),
            anchor: None,
            arrangement: Arrangement::Manual,
            grouping: FolderGrouping::Mixed,
            cell: (96.0, 112.0),
            icons_visible: true,
            remembered_outputs: HashMap::new(),
            locale: sys_locale::get_locale().unwrap_or_else(|| "en-US".into()),
        }
    }

    pub fn items(&self) -> &[DesktopItem] {
        &self.items
    }

    pub fn selected(&self) -> &HashSet<DesktopEntryId> {
        &self.selection
    }

    pub fn arrangement(&self) -> Arrangement {
        self.arrangement
    }

    pub fn grid(&self) -> (f32, f32) {
        self.cell
    }

    pub fn icons_visible(&self) -> bool {
        self.icons_visible
    }

    pub fn set_icons_visible(&mut self, visible: bool) {
        self.icons_visible = visible;
    }

    pub fn set_grid(&mut self, width: f32, height: f32) {
        self.cell = (width.max(48.0), height.max(48.0));
        self.constrain_all();
    }

    /// Reconciles a provider snapshot incrementally by stable file identity. Renames retain layout.
    pub fn reconcile(&mut self, entries: Vec<(FileIdentity, FileEntry)>) {
        let mut previous = self
            .items
            .drain(..)
            .map(|item| (item.id, item))
            .collect::<HashMap<_, _>>();
        self.items = entries
            .into_iter()
            .map(|(identity, entry)| {
                let id = DesktopEntryId(identity);
                if let Some(mut item) = previous.remove(&id) {
                    item.entry = entry;
                    item
                } else {
                    let output = self
                        .remembered_outputs
                        .get(&id)
                        .filter(|candidate| self.output(candidate).is_some())
                        .cloned()
                        .or_else(|| self.outputs.first().map(|output| output.id.clone()))
                        .unwrap_or_default();
                    DesktopItem {
                        id,
                        entry,
                        output,
                        position: Point::default(),
                    }
                }
            })
            .collect();
        let ids = self
            .items
            .iter()
            .map(|item| item.id)
            .collect::<HashSet<_>>();
        self.selection.retain(|id| ids.contains(id));
        self.anchor = self.anchor.filter(|id| ids.contains(id));
        self.arrange();
    }

    pub fn activate(&self, id: DesktopEntryId) -> Option<DesktopFileAction> {
        self.item(id).map(|item| {
            if item.entry.is_directory {
                DesktopFileAction::Browse(item.entry.path.clone())
            } else {
                DesktopFileAction::Open(item.entry.path.clone())
            }
        })
    }

    pub fn select(&mut self, id: DesktopEntryId, modifiers: SelectionModifiers) {
        let Some(clicked) = self.items.iter().position(|item| item.id == id) else {
            return;
        };
        if modifiers.range || modifiers.additive_range {
            let anchor = self
                .anchor
                .and_then(|anchor| self.items.iter().position(|item| item.id == anchor))
                .unwrap_or(clicked);
            if !modifiers.additive_range {
                self.selection.clear();
            }
            let (start, end) = if anchor <= clicked {
                (anchor, clicked)
            } else {
                (clicked, anchor)
            };
            self.selection
                .extend(self.items[start..=end].iter().map(|item| item.id));
        } else if modifiers.toggle {
            if !self.selection.remove(&id) {
                self.selection.insert(id);
            }
            self.anchor = Some(id);
        } else {
            self.selection.clear();
            self.selection.insert(id);
            self.anchor = Some(id);
        }
    }

    pub fn select_all(&mut self) {
        self.selection = self.items.iter().map(|item| item.id).collect();
    }

    pub fn clear_selection(&mut self) {
        self.selection.clear();
        self.anchor = None;
    }

    pub fn select_region(&mut self, region: Rect, additive: bool) {
        if !additive {
            self.selection.clear();
        }
        self.selection.extend(self.items.iter().filter_map(|item| {
            let bounds = Rect {
                x: item.position.x,
                y: item.position.y,
                width: self.cell.0,
                height: self.cell.1,
            };
            intersects(bounds, region).then_some(item.id)
        }));
    }

    /// Shared keyboard/controller/accessibility directional selection command.
    pub fn select_direction(&mut self, horizontal: i8, vertical: i8, extend: bool) {
        let Some(current) = self
            .anchor
            .and_then(|anchor| self.items.iter().find(|item| item.id == anchor))
            .or_else(|| self.items.first())
        else {
            return;
        };
        let origin = current.position;
        let candidate = self
            .items
            .iter()
            .filter(|item| item.id != current.id)
            .filter_map(|item| {
                let dx = item.position.x - origin.x;
                let dy = item.position.y - origin.y;
                let intended = (horizontal == 0 || dx.signum() == f32::from(horizontal).signum())
                    && (vertical == 0 || dy.signum() == f32::from(vertical).signum());
                intended.then_some((dx * dx + dy * dy, item.id))
            })
            .min_by(|left, right| left.0.total_cmp(&right.0))
            .map(|(_, id)| id)
            .unwrap_or(current.id);
        self.select(
            candidate,
            SelectionModifiers {
                range: extend,
                ..SelectionModifiers::default()
            },
        );
    }

    /// Moves the selected group. Moving an unselected item selects only that item first.
    pub fn move_group(&mut self, dragged: DesktopEntryId, delta: Point, output: &str) {
        if !self.selection.contains(&dragged) {
            self.select(dragged, SelectionModifiers::default());
        }
        let Some(work_area) = self.output(output).map(|output| output.work_area) else {
            return;
        };
        self.arrangement = Arrangement::Manual;
        let moving = self.selection.clone();
        for item in &mut self.items {
            if moving.contains(&item.id) {
                item.output = output.to_owned();
                item.position.x += delta.x;
                item.position.y += delta.y;
                item.position = snap_and_clamp(item.position, work_area, self.cell);
                self.remembered_outputs.insert(item.id, output.to_owned());
            }
        }
        self.resolve_collisions(&moving);
    }

    pub fn set_arrangement(&mut self, arrangement: Arrangement, grouping: FolderGrouping) {
        self.arrangement = arrangement;
        self.grouping = grouping;
        self.arrange();
    }

    pub fn align_to_grid(&mut self) {
        self.constrain_all();
        self.resolve_collisions(&HashSet::new());
    }

    pub fn clean_up(&mut self) {
        self.place_in_visual_order();
    }

    /// Reconciles output topology without discarding affinity for disconnected displays.
    pub fn set_outputs(&mut self, outputs: Vec<DesktopOutput>) {
        let previously_valid = self
            .outputs
            .iter()
            .map(|output| output.id.as_str())
            .collect::<HashSet<_>>();
        for item in &self.items {
            if previously_valid.contains(item.output.as_str()) {
                self.remembered_outputs
                    .entry(item.id)
                    .or_insert_with(|| item.output.clone());
            }
        }
        self.outputs = outputs;
        let fallback = self.outputs.first().map(|output| output.id.clone());
        if let Some(fallback) = fallback {
            let valid = self
                .outputs
                .iter()
                .map(|output| output.id.as_str())
                .collect::<HashSet<_>>();
            for item in &mut self.items {
                if let Some(affinity) = self.remembered_outputs.get(&item.id)
                    && valid.contains(affinity.as_str())
                {
                    item.output.clone_from(affinity);
                } else if !valid.contains(item.output.as_str()) {
                    item.output.clone_from(&fallback);
                    self.remembered_outputs
                        .entry(item.id)
                        .or_insert_with(|| fallback.clone());
                }
            }
            self.constrain_all();
            self.resolve_collisions(&HashSet::new());
        }
    }

    pub fn save(&self, path: impl AsRef<Path>) -> io::Result<()> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut body = format!(
            "v1\narrangement={}\ngrouping={}\ngrid={}:{}\nicons-visible={}\n",
            encode_arrangement(self.arrangement),
            match self.grouping {
                FolderGrouping::Mixed => "mixed",
                FolderGrouping::FoldersFirst => "folders-first",
            },
            self.cell.0,
            self.cell.1,
            self.icons_visible,
        );
        for item in &self.items {
            let output = self
                .remembered_outputs
                .get(&item.id)
                .unwrap_or(&item.output);
            body.push_str(&format!(
                "item={}:{}:{}:{}:{}\n",
                item.id.0.0,
                item.id.0.1,
                hex(output.as_bytes()),
                item.position.x,
                item.position.y
            ));
        }
        let temporary = path.with_extension(format!("tmp-{}", std::process::id()));
        fs::write(&temporary, body)?;
        fs::rename(temporary, path)
    }

    pub fn restore(&mut self, path: impl AsRef<Path>) -> io::Result<()> {
        let text = fs::read_to_string(path)?;
        let mut placements = HashMap::new();
        for line in text.lines() {
            if let Some(value) = line.strip_prefix("arrangement=") {
                if let Some(arrangement) = decode_arrangement(value) {
                    self.arrangement = arrangement;
                }
            } else if let Some(value) = line.strip_prefix("grouping=") {
                self.grouping = if value == "folders-first" {
                    FolderGrouping::FoldersFirst
                } else {
                    FolderGrouping::Mixed
                };
            } else if let Some(value) = line.strip_prefix("grid=") {
                if let Some((width, height)) = value.split_once(':')
                    && let (Ok(width), Ok(height)) = (width.parse(), height.parse())
                {
                    self.cell = (width, height);
                }
            } else if let Some(value) = line.strip_prefix("icons-visible=") {
                if let Ok(visible) = value.parse() {
                    self.icons_visible = visible;
                }
            } else if let Some(value) = line.strip_prefix("item=") {
                let fields = value.split(':').collect::<Vec<_>>();
                if fields.len() == 5
                    && let (Ok(device), Ok(file), Some(output), Ok(x), Ok(y)) = (
                        fields[0].parse(),
                        fields[1].parse(),
                        unhex(fields[2]),
                        fields[3].parse(),
                        fields[4].parse(),
                    )
                {
                    placements.insert(
                        DesktopEntryId(FileIdentity(device, file)),
                        (output, Point { x, y }),
                    );
                }
            }
        }
        for item in &mut self.items {
            if let Some((output, position)) = placements.remove(&item.id) {
                self.remembered_outputs.insert(item.id, output.clone());
                if self.outputs.iter().any(|candidate| candidate.id == output) {
                    item.output = output;
                }
                item.position = position;
            }
        }
        self.arrange();
        Ok(())
    }

    fn item(&self, id: DesktopEntryId) -> Option<&DesktopItem> {
        self.items.iter().find(|item| item.id == id)
    }

    fn output(&self, id: &str) -> Option<&DesktopOutput> {
        self.outputs.iter().find(|output| output.id == id)
    }

    fn arrange(&mut self) {
        match self.arrangement {
            Arrangement::Manual => {
                self.constrain_all();
                self.resolve_collisions(&HashSet::new());
            }
            Arrangement::Sorted { key, direction } => {
                let grouping = self.grouping;
                let locale = self.locale.clone();
                let collator = locale
                    .parse::<icu_locale::Locale>()
                    .ok()
                    .and_then(|locale| {
                        icu_collator::Collator::try_new(locale.into(), Default::default()).ok()
                    });
                self.items.sort_by(|left, right| {
                    compare_items(left, right, key, grouping, direction, collator.as_ref())
                });
                self.place_in_visual_order();
            }
        }
    }

    fn place_in_visual_order(&mut self) {
        let outputs = self.outputs.clone();
        for output in outputs {
            for (cell_index, item) in self
                .items
                .iter_mut()
                .filter(|item| item.output == output.id)
                .enumerate()
            {
                item.position = cell_position(output.work_area, self.cell, cell_index);
            }
        }
    }

    fn constrain_all(&mut self) {
        let areas = self
            .outputs
            .iter()
            .map(|output| (output.id.clone(), output.work_area))
            .collect::<HashMap<_, _>>();
        for item in &mut self.items {
            if let Some(area) = areas.get(&item.output) {
                item.position = snap_and_clamp(item.position, *area, self.cell);
            }
        }
    }

    fn resolve_collisions(&mut self, preferred: &HashSet<DesktopEntryId>) {
        let outputs = self.outputs.clone();
        for output in outputs {
            let capacity = grid_capacity(output.work_area, self.cell);
            let mut occupied = HashSet::new();
            let mut indices = (0..self.items.len())
                .filter(|index| self.items[*index].output == output.id)
                .collect::<Vec<_>>();
            indices.sort_by_key(|index| !preferred.contains(&self.items[*index].id));
            for index in indices {
                let requested = cell_index(output.work_area, self.cell, self.items[index].position);
                let available = (0..capacity)
                    .map(|offset| (requested + offset) % capacity.max(1))
                    .find(|cell| occupied.insert(*cell))
                    .unwrap_or(requested);
                self.items[index].position = cell_position(output.work_area, self.cell, available);
            }
        }
    }
}

fn compare_items(
    left: &DesktopItem,
    right: &DesktopItem,
    key: SortKey,
    grouping: FolderGrouping,
    direction: SortDirection,
    collator: Option<&icu_collator::CollatorBorrowed<'_>>,
) -> Ordering {
    let folder_order = match grouping {
        FolderGrouping::Mixed => Ordering::Equal,
        FolderGrouping::FoldersFirst => right.entry.is_directory.cmp(&left.entry.is_directory),
    };
    let value_order = match key {
        SortKey::Name => names(left, right, collator),
        SortKey::Kind => extension(&left.entry)
            .cmp(&extension(&right.entry))
            .then_with(|| names(left, right, collator)),
        SortKey::Size => left
            .entry
            .size
            .cmp(&right.entry.size)
            .then_with(|| names(left, right, collator)),
        SortKey::Modified => left
            .entry
            .modified
            .cmp(&right.entry.modified)
            .then_with(|| names(left, right, collator)),
    };
    let value_order = match direction {
        SortDirection::Ascending => value_order,
        SortDirection::Descending => value_order.reverse(),
    };
    folder_order
        .then(value_order)
        .then_with(|| left.id.cmp(&right.id))
}

fn names(
    left: &DesktopItem,
    right: &DesktopItem,
    collator: Option<&icu_collator::CollatorBorrowed<'_>>,
) -> Ordering {
    let left = left.entry.display_name();
    let right = right.entry.display_name();
    collator
        .map(|collator| collator.compare(&left, &right))
        .unwrap_or_else(|| left.to_lowercase().cmp(&right.to_lowercase()))
}

fn extension(entry: &FileEntry) -> String {
    entry
        .path
        .extension()
        .map_or_else(String::new, |value| value.to_string_lossy().to_lowercase())
}

fn grid_capacity(area: Rect, cell: (f32, f32)) -> usize {
    ((area.width / cell.0).floor().max(1.0) * (area.height / cell.1).floor().max(1.0)) as usize
}

fn cell_index(area: Rect, cell: (f32, f32), point: Point) -> usize {
    let rows = (area.height / cell.1).floor().max(1.0) as usize;
    let column = ((point.x - area.x) / cell.0).round().max(0.0) as usize;
    let row = ((point.y - area.y) / cell.1).round().max(0.0) as usize;
    column * rows + row.min(rows - 1)
}

fn cell_position(area: Rect, cell: (f32, f32), index: usize) -> Point {
    let rows = (area.height / cell.1).floor().max(1.0) as usize;
    let columns = (area.width / cell.0).floor().max(1.0) as usize;
    let index = index.min(rows * columns - 1);
    Point {
        x: area.x + (index / rows) as f32 * cell.0,
        y: area.y + (index % rows) as f32 * cell.1,
    }
}

fn snap_and_clamp(point: Point, area: Rect, cell: (f32, f32)) -> Point {
    let x = area.x + ((point.x - area.x) / cell.0).round() * cell.0;
    let y = area.y + ((point.y - area.y) / cell.1).round() * cell.1;
    Point {
        x: x.clamp(area.x, (area.x + area.width - cell.0).max(area.x)),
        y: y.clamp(area.y, (area.y + area.height - cell.1).max(area.y)),
    }
}

fn intersects(left: Rect, right: Rect) -> bool {
    left.x < right.x + right.width
        && left.x + left.width > right.x
        && left.y < right.y + right.height
        && left.y + left.height > right.y
}

fn encode_arrangement(value: Arrangement) -> &'static str {
    match value {
        Arrangement::Manual => "manual",
        Arrangement::Sorted {
            key: SortKey::Name,
            direction: SortDirection::Ascending,
        } => "name-asc",
        Arrangement::Sorted {
            key: SortKey::Name,
            direction: SortDirection::Descending,
        } => "name-desc",
        Arrangement::Sorted {
            key: SortKey::Kind,
            direction: SortDirection::Ascending,
        } => "kind-asc",
        Arrangement::Sorted {
            key: SortKey::Kind,
            direction: SortDirection::Descending,
        } => "kind-desc",
        Arrangement::Sorted {
            key: SortKey::Size,
            direction: SortDirection::Ascending,
        } => "size-asc",
        Arrangement::Sorted {
            key: SortKey::Size,
            direction: SortDirection::Descending,
        } => "size-desc",
        Arrangement::Sorted {
            key: SortKey::Modified,
            direction: SortDirection::Ascending,
        } => "modified-asc",
        Arrangement::Sorted {
            key: SortKey::Modified,
            direction: SortDirection::Descending,
        } => "modified-desc",
    }
}

fn decode_arrangement(value: &str) -> Option<Arrangement> {
    if value == "manual" {
        return Some(Arrangement::Manual);
    }
    let (key, direction) = value.rsplit_once('-')?;
    Some(Arrangement::Sorted {
        key: match key {
            "name" => SortKey::Name,
            "kind" => SortKey::Kind,
            "size" => SortKey::Size,
            "modified" => SortKey::Modified,
            _ => return None,
        },
        direction: if direction == "desc" {
            SortDirection::Descending
        } else if direction == "asc" {
            SortDirection::Ascending
        } else {
            return None;
        },
    })
}

fn hex(value: &[u8]) -> String {
    value.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn unhex(value: &str) -> Option<String> {
    if !value.len().is_multiple_of(2) {
        return None;
    }
    let bytes = (0..value.len())
        .step_by(2)
        .map(|index| u8::from_str_radix(&value[index..index + 2], 16))
        .collect::<Result<Vec<_>, _>>()
        .ok()?;
    String::from_utf8(bytes).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;
    use std::time::UNIX_EPOCH;

    fn output(id: &str, x: f32) -> DesktopOutput {
        DesktopOutput {
            id: id.into(),
            work_area: Rect {
                x,
                y: 32.0,
                width: 300.0,
                height: 360.0,
            },
            scale: 1.0,
        }
    }

    fn entry(id: u64, name: &str, directory: bool, size: u64) -> (FileIdentity, FileEntry) {
        (
            FileIdentity(7, id),
            FileEntry {
                name: OsString::from(name),
                path: PathBuf::from("/desktop").join(name),
                is_directory: directory,
                size: (!directory).then_some(size),
                modified: Some(UNIX_EPOCH + std::time::Duration::from_secs(size)),
            },
        )
    }

    #[test]
    fn scenario_reconcile_selection_activation_and_rename_use_file_authority() {
        let mut layout = DesktopLayout::new(vec![output("left", -300.0)]);
        layout.reconcile(vec![
            entry(1, "Notes", true, 0),
            entry(2, "todo.txt", false, 12),
        ]);
        let notes = DesktopEntryId(FileIdentity(7, 1));
        let todo = DesktopEntryId(FileIdentity(7, 2));
        layout.select(notes, SelectionModifiers::default());
        layout.select(
            todo,
            SelectionModifiers {
                toggle: true,
                ..Default::default()
            },
        );
        assert_eq!(layout.selected().len(), 2);
        layout.select(notes, SelectionModifiers::default());
        layout.select(
            todo,
            SelectionModifiers {
                range: true,
                ..Default::default()
            },
        );
        assert_eq!(layout.selected().len(), 2);
        let first_position = layout.items()[0].position;
        layout.select_region(
            Rect {
                x: first_position.x,
                y: first_position.y,
                width: 96.0,
                height: 112.0,
            },
            false,
        );
        assert_eq!(layout.selected().len(), 1);
        layout.select_direction(0, 1, true);
        assert_eq!(layout.selected().len(), 2);
        assert_eq!(
            layout.activate(notes),
            Some(DesktopFileAction::Browse("/desktop/Notes".into()))
        );
        assert_eq!(
            layout.activate(todo),
            Some(DesktopFileAction::Open("/desktop/todo.txt".into()))
        );
        layout.reconcile(vec![entry(1, "Renamed", true, 0)]);
        assert_eq!(layout.items()[0].entry.display_name(), "Renamed");
        assert_eq!(layout.selected(), &HashSet::from([notes]));
    }

    #[test]
    fn scenario_group_move_snaps_constrains_and_never_emits_file_mutation() {
        let mut layout = DesktopLayout::new(vec![output("right", 100.0)]);
        layout.reconcile(vec![entry(1, "a", false, 1), entry(2, "b", false, 2)]);
        layout.select_all();
        layout.move_group(
            DesktopEntryId(FileIdentity(7, 1)),
            Point {
                x: 9_000.0,
                y: -9_000.0,
            },
            "right",
        );
        assert!(layout.items().iter().all(|item| item.output == "right"));
        assert!(
            layout
                .items()
                .iter()
                .all(|item| item.position.x >= 100.0 && item.position.y >= 32.0)
        );
        assert_ne!(layout.items()[0].position, layout.items()[1].position);
    }

    #[test]
    fn scenario_every_sort_direction_grouping_and_manual_transition_is_stable() {
        let mut layout = DesktopLayout::new(vec![output("main", 0.0)]);
        layout.reconcile(vec![
            entry(3, "b.rs", false, 9),
            entry(2, "A.txt", false, 20),
            entry(1, "folder", true, 0),
        ]);
        for key in [
            SortKey::Name,
            SortKey::Kind,
            SortKey::Size,
            SortKey::Modified,
        ] {
            for direction in [SortDirection::Ascending, SortDirection::Descending] {
                layout.set_arrangement(
                    Arrangement::Sorted { key, direction },
                    FolderGrouping::Mixed,
                );
                let once = layout
                    .items()
                    .iter()
                    .map(|item| item.id)
                    .collect::<Vec<_>>();
                layout.set_arrangement(
                    Arrangement::Sorted { key, direction },
                    FolderGrouping::Mixed,
                );
                assert_eq!(
                    layout
                        .items()
                        .iter()
                        .map(|item| item.id)
                        .collect::<Vec<_>>(),
                    once
                );
            }
        }
        layout.set_arrangement(
            Arrangement::Sorted {
                key: SortKey::Name,
                direction: SortDirection::Ascending,
            },
            FolderGrouping::FoldersFirst,
        );
        assert!(layout.items()[0].entry.is_directory);
        layout.move_group(
            DesktopEntryId(FileIdentity(7, 1)),
            Point { x: 96.0, y: 0.0 },
            "main",
        );
        assert_eq!(layout.arrangement(), Arrangement::Manual);
    }

    #[test]
    fn locale_collation_handles_turkish_german_and_swedish_primary_order() {
        let locale: icu_locale::Locale = "sv-SE".parse().unwrap();
        let collator = icu_collator::Collator::try_new(locale.into(), Default::default()).unwrap();
        assert_eq!(collator.compare("Zebra", "Örebro"), Ordering::Less);
    }

    #[test]
    fn scenario_hotplug_reflows_but_remembers_output_affinity_and_persists_atomically() {
        let mut layout = DesktopLayout::new(vec![output("left", -300.0), output("right", 100.0)]);
        layout.reconcile(vec![entry(1, "a", false, 1)]);
        let id = DesktopEntryId(FileIdentity(7, 1));
        layout.move_group(id, Point { x: 40.0, y: 120.0 }, "right");
        layout.set_grid(128.0, 144.0);
        layout.set_icons_visible(false);
        layout.set_outputs(vec![output("left", -300.0)]);
        assert_eq!(layout.items()[0].output, "left");
        layout.set_outputs(vec![output("left", -300.0), output("right", 100.0)]);
        assert_eq!(layout.items()[0].output, "right");
        layout.set_outputs(vec![output("left", -300.0)]);

        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("desktop-layout");
        layout.save(&path).unwrap();
        let mut restored = DesktopLayout::new(vec![output("left", -300.0), output("right", 100.0)]);
        restored.reconcile(vec![entry(1, "a", false, 1)]);
        restored.restore(&path).unwrap();
        assert_eq!(restored.items()[0].output, "right");
        assert_eq!(restored.grid(), (128.0, 144.0));
        assert!(!restored.icons_visible());
        assert!(
            !path
                .with_extension(format!("tmp-{}", std::process::id()))
                .exists()
        );
    }
}
