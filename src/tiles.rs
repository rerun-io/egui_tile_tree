use egui::{Pos2, Rect};

use super::{
    Behavior, Container, ContainerInsertion, ContainerKind, GcAction, Grid, InsertionPoint, Linear,
    LinearDir, SimplificationOptions, SimplifyAction, Tabs, Tile, TileId,
};

/// Contains all tile state, but no root.
///
/// ```
/// use egui_tiles::{Tiles, TileId, Tree};
///
/// struct Pane { } // put some state here
///
/// let mut tiles = Tiles::default();
/// let tabs: Vec<TileId> = vec![tiles.insert_pane(Pane { }), tiles.insert_pane(Pane { })];
/// let root: TileId = tiles.insert_tab_tile(tabs);
///
/// let tree = Tree::new(root, tiles);
/// ```
#[derive(Clone, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
pub struct Tiles<Pane> {
    next_tile_id: u64,

    tiles: ahash::HashMap<TileId, Tile<Pane>>,

    /// Tiles are visible by default, so we only store the invisible ones.
    invisible: ahash::HashSet<TileId>,

    /// Filled in by the layout step at the start of each frame.
    #[cfg_attr(feature = "serde", serde(default, skip))]
    pub(super) rects: ahash::HashMap<TileId, Rect>,
}

impl<Pane: PartialEq> PartialEq for Tiles<Pane> {
    fn eq(&self, other: &Tiles<Pane>) -> bool {
        let Self {
            next_tile_id: _, // ignored
            tiles,
            invisible,
            rects: _, // ignore transient state
        } = self;
        tiles == &other.tiles && invisible == &other.invisible
    }
}

impl<Pane> Default for Tiles<Pane> {
    fn default() -> Self {
        Self {
            next_tile_id: 1,
            tiles: Default::default(),
            invisible: Default::default(),
            rects: Default::default(),
        }
    }
}

// ----------------------------------------------------------------------------

impl<Pane> Tiles<Pane> {
    pub(super) fn try_rect(&self, tile_id: TileId) -> Option<Rect> {
        if self.is_visible(tile_id) {
            self.rects.get(&tile_id).copied()
        } else {
            None
        }
    }

    pub(super) fn rect(&self, tile_id: TileId) -> Rect {
        let rect = self.try_rect(tile_id);
        debug_assert!(rect.is_some(), "Failed to find rect for {tile_id:?}");
        rect.unwrap_or(egui::Rect::from_min_max(Pos2::ZERO, Pos2::ZERO))
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.tiles.is_empty()
    }

    /// The number of tiles, including invisible tiles.
    #[inline]
    pub fn len(&self) -> usize {
        self.tiles.len()
    }

    pub fn get(&self, tile_id: TileId) -> Option<&Tile<Pane>> {
        self.tiles.get(&tile_id)
    }

    pub fn get_mut(&mut self, tile_id: TileId) -> Option<&mut Tile<Pane>> {
        self.tiles.get_mut(&tile_id)
    }

    /// All tiles, in arbitrary order
    pub fn iter(&self) -> impl Iterator<Item = (&TileId, &Tile<Pane>)> + '_ {
        self.tiles.iter()
    }

    /// All tiles, in arbitrary order
    pub fn iter_mut(&mut self) -> impl Iterator<Item = (&TileId, &mut Tile<Pane>)> + '_ {
        self.tiles.iter_mut()
    }

    /// All [`TileId`]s, in arbitrary order
    pub fn tile_ids(&self) -> impl Iterator<Item = TileId> + '_ {
        self.tiles.keys().copied()
    }

    /// All [`Tile`]s in arbitrary order
    pub fn tiles(&self) -> impl Iterator<Item = &Tile<Pane>> + '_ {
        self.tiles.values()
    }

    /// All [`Tile`]s in arbitrary order
    pub fn tiles_mut(&mut self) -> impl Iterator<Item = &mut Tile<Pane>> + '_ {
        self.tiles.values_mut()
    }

    /// Tiles are visible by default.
    ///
    /// Invisible tiles still retain their place in the tile hierarchy.
    pub fn is_visible(&self, tile_id: TileId) -> bool {
        !self.invisible.contains(&tile_id)
    }

    /// Tiles are visible by default.
    ///
    /// Invisible tiles still retain their place in the tile hierarchy.
    pub fn set_visible(&mut self, tile_id: TileId, visible: bool) {
        if visible {
            self.invisible.remove(&tile_id);
        } else {
            self.invisible.insert(tile_id);
        }
    }

    pub fn toggle_visibility(&mut self, tile_id: TileId) {
        self.set_visible(tile_id, !self.is_visible(tile_id));
    }

    pub fn insert(&mut self, id: TileId, tile: Tile<Pane>) {
        self.tiles.insert(id, tile);
    }

    pub fn remove(&mut self, id: TileId) -> Option<Tile<Pane>> {
        self.tiles.remove(&id)
    }

    /// Remove the given tile and all child tiles, recursively.
    ///
    /// All removed tiles are returned in unspecified order.
    pub fn remove_recursively(&mut self, id: TileId) -> Vec<Tile<Pane>> {
        let mut removed_tiles = vec![];
        self.remove_recursively_impl(id, &mut removed_tiles);
        removed_tiles
    }

    fn remove_recursively_impl(&mut self, id: TileId, removed_tiles: &mut Vec<Tile<Pane>>) {
        if let Some(tile) = self.remove(id) {
            if let Tile::Container(container) = &tile {
                for &child_id in container.children() {
                    self.remove_recursively_impl(child_id, removed_tiles);
                }
            }
            removed_tiles.push(tile);
        }
    }

    #[must_use]
    pub fn insert_new(&mut self, tile: Tile<Pane>) -> TileId {
        let id = TileId::from_u64(self.next_tile_id);
        self.next_tile_id += 1;
        self.tiles.insert(id, tile);
        id
    }

    #[must_use]
    pub fn insert_pane(&mut self, pane: Pane) -> TileId {
        self.insert_new(Tile::Pane(pane))
    }

    #[must_use]
    pub fn insert_container(&mut self, container: impl Into<Container>) -> TileId {
        self.insert_new(Tile::Container(container.into()))
    }

    #[must_use]
    pub fn insert_tab_tile(&mut self, children: Vec<TileId>) -> TileId {
        self.insert_new(Tile::Container(Container::new_tabs(children)))
    }

    #[must_use]
    pub fn insert_horizontal_tile(&mut self, children: Vec<TileId>) -> TileId {
        self.insert_new(Tile::Container(Container::new_linear(
            LinearDir::Horizontal,
            children,
        )))
    }

    #[must_use]
    pub fn insert_vertical_tile(&mut self, children: Vec<TileId>) -> TileId {
        self.insert_new(Tile::Container(Container::new_linear(
            LinearDir::Vertical,
            children,
        )))
    }

    #[must_use]
    pub fn insert_grid_tile(&mut self, children: Vec<TileId>) -> TileId {
        self.insert_new(Tile::Container(Container::new_grid(children)))
    }

    pub fn parent_of(&self, child_id: TileId) -> Option<TileId> {
        for (tile_id, tile) in &self.tiles {
            if let Tile::Container(container) = tile {
                if container.has_child(child_id) {
                    return Some(*tile_id);
                }
            }
        }
        None
    }

    pub fn is_root(&self, tile_id: TileId) -> bool {
        self.parent_of(tile_id).is_none()
    }

    pub(super) fn insert_at(&mut self, insertion_point: InsertionPoint, inserted_id: TileId) {
        let InsertionPoint {
            parent_id,
            insertion,
        } = insertion_point;

        let Some(mut parent_tile) = self.tiles.remove(&parent_id) else {
            log::warn!("Failed to insert: could not find parent {parent_id:?}");
            return;
        };

        match insertion {
            ContainerInsertion::Tabs(index) => {
                if let Tile::Container(Container::Tabs(tabs)) = &mut parent_tile {
                    let index = index.min(tabs.children.len());
                    tabs.children.insert(index, inserted_id);
                    tabs.set_active(inserted_id);
                    self.tiles.insert(parent_id, parent_tile);
                } else {
                    let new_tile_id = self.insert_new(parent_tile);
                    let mut tabs = Tabs::new(vec![new_tile_id]);
                    tabs.children.insert(index.min(1), inserted_id);
                    tabs.set_active(inserted_id);
                    self.tiles
                        .insert(parent_id, Tile::Container(Container::Tabs(tabs)));
                }
            }
            ContainerInsertion::Horizontal(index) => {
                if let Tile::Container(Container::Linear(Linear {
                    dir: LinearDir::Horizontal,
                    children,
                    ..
                })) = &mut parent_tile
                {
                    let index = index.min(children.len());
                    children.insert(index, inserted_id);
                    self.tiles.insert(parent_id, parent_tile);
                } else {
                    let new_tile_id = self.insert_new(parent_tile);
                    let mut linear = Linear::new(LinearDir::Horizontal, vec![new_tile_id]);
                    linear.children.insert(index.min(1), inserted_id);
                    self.tiles
                        .insert(parent_id, Tile::Container(Container::Linear(linear)));
                }
            }
            ContainerInsertion::Vertical(index) => {
                if let Tile::Container(Container::Linear(Linear {
                    dir: LinearDir::Vertical,
                    children,
                    ..
                })) = &mut parent_tile
                {
                    let index = index.min(children.len());
                    children.insert(index, inserted_id);
                    self.tiles.insert(parent_id, parent_tile);
                } else {
                    let new_tile_id = self.insert_new(parent_tile);
                    let mut linear = Linear::new(LinearDir::Vertical, vec![new_tile_id]);
                    linear.children.insert(index.min(1), inserted_id);
                    self.tiles
                        .insert(parent_id, Tile::Container(Container::Linear(linear)));
                }
            }
            ContainerInsertion::Grid(index) => {
                if let Tile::Container(Container::Grid(grid)) = &mut parent_tile {
                    grid.insert_at(index, inserted_id);
                    self.tiles.insert(parent_id, parent_tile);
                } else {
                    let new_tile_id = self.insert_new(parent_tile);
                    let grid = Grid::new(vec![new_tile_id, inserted_id]);
                    self.tiles
                        .insert(parent_id, Tile::Container(Container::Grid(grid)));
                }
            }
        }
    }

    /// Detect cycles, duplications, and other invalid state, and fix it.
    ///
    /// Will also call [`Behavior::retain_pane`] to check if a users wants to remove a pane.
    ///
    /// Finally free up any tiles that are no longer reachable from the root.
    pub(super) fn gc_roots(&mut self, behavior: &mut dyn Behavior<Pane>, roots: &[TileId]) {
        let mut visited = Default::default();

        for &root_id in roots {
            // We ignore the returned root action, because we will never remove the root.
            let _root_action = self.gc_tile_id(behavior, &mut visited, root_id);
        }

        if visited.len() < self.tiles.len() {
            // This can happen if a user coses a whole tree of viewports,
            // e.g. when closing a viewport tile.
            log::debug!(
                "GC collecting tiles: {:?}",
                self.tiles
                    .keys()
                    .filter(|id| !visited.contains(id))
                    .collect::<Vec<_>>()
            );
        }

        self.invisible.retain(|tile_id| visited.contains(tile_id));
        self.tiles.retain(|tile_id, _| visited.contains(tile_id));
    }

    /// Detect cycles, duplications, and other invalid state, and remove them.
    fn gc_tile_id(
        &mut self,
        behavior: &mut dyn Behavior<Pane>,
        visited: &mut ahash::HashSet<TileId>,
        tile_id: TileId,
    ) -> GcAction {
        let Some(mut tile) = self.tiles.remove(&tile_id) else { return GcAction::Remove; };
        if !visited.insert(tile_id) {
            log::warn!("Cycle or duplication detected");
            return GcAction::Remove;
        }

        match &mut tile {
            Tile::Pane(pane) => {
                if !behavior.retain_pane(pane) {
                    return GcAction::Remove;
                }
            }
            Tile::Container(container) => {
                container
                    .retain(|child| self.gc_tile_id(behavior, visited, child) == GcAction::Keep);
            }
        }
        self.tiles.insert(tile_id, tile);
        GcAction::Keep
    }

    pub(super) fn layout_tile(
        &mut self,
        style: &egui::Style,
        behavior: &mut dyn Behavior<Pane>,
        rect: Rect,
        tile_id: TileId,
    ) {
        let Some(mut tile) = self.tiles.remove(&tile_id) else {
            log::warn!("Failed to find tile {tile_id:?} during layout");
            return;
        };
        self.rects.insert(tile_id, rect);

        if let Tile::Container(container) = &mut tile {
            container.layout(self, style, behavior, rect);
        }

        self.tiles.insert(tile_id, tile);
    }

    /// Simplify the tree, perhaps culling empty containers,
    /// and/or merging single-child containers into their parent.
    ///
    /// Drag-dropping tiles can often leave containers empty, or with only a single child.
    /// This is often undersired, so this function can be used to clean up the tree.
    ///
    /// What simplifcations are allowed is controlled by the [`SimplificationOptions`].
    pub(super) fn simplify(
        &mut self,
        options: &SimplificationOptions,
        it: TileId,
        parent_kind: Option<ContainerKind>,
    ) -> SimplifyAction {
        let Some(mut tile) = self.tiles.remove(&it) else {
            log::warn!("Failed to find tile {it:?} during simplify");
            return SimplifyAction::Remove;
        };

        if let Tile::Container(container) = &mut tile {
            let kind = container.kind();
            container.simplify_children(|child| self.simplify(options, child, Some(kind)));

            if kind == ContainerKind::Tabs {
                if options.prune_empty_tabs && container.is_empty() {
                    log::trace!("Simplify: removing empty tabs container");
                    return SimplifyAction::Remove;
                }

                if options.prune_single_child_tabs {
                    if let Some(only_child) = container.only_child() {
                        let child_is_pane = matches!(self.get(only_child), Some(Tile::Pane(_)));

                        if options.all_panes_must_have_tabs
                            && child_is_pane
                            && parent_kind != Some(ContainerKind::Tabs)
                        {
                            // Keep it, even though we only have one child
                        } else {
                            log::trace!("Simplify: collapsing single-child tabs container");
                            return SimplifyAction::Replace(only_child);
                        }
                    }
                }
            } else {
                if options.join_nested_linear_containerss {
                    if let Container::Linear(parent) = container {
                        let mut new_children = Vec::with_capacity(parent.children.len());
                        for child_id in parent.children.drain(..) {
                            if let Some(Tile::Container(Container::Linear(child))) =
                                &mut self.get_mut(child_id)
                            {
                                if parent.dir == child.dir {
                                    // absorb the child
                                    log::trace!(
                                        "Simplify: absorbing nested linear container with {} children",
                                        child.children.len()
                                    );

                                    let mut child_share_sum = 0.0;
                                    for &grandchild in &child.children {
                                        child_share_sum += child.shares[grandchild];
                                    }
                                    let share_normalizer =
                                        parent.shares[child_id] / child_share_sum;
                                    for &grandchild in &child.children {
                                        new_children.push(grandchild);
                                        parent.shares[grandchild] =
                                            child.shares[grandchild] * share_normalizer;
                                    }

                                    self.tiles.remove(&child_id);
                                } else {
                                    // keep the child
                                    new_children.push(child_id);
                                }
                            } else {
                                new_children.push(child_id);
                            }
                        }
                        parent.children = new_children;
                    }
                }

                if options.prune_empty_containers && container.is_empty() {
                    log::trace!("Simplify: removing empty container tile");
                    return SimplifyAction::Remove;
                }
                if options.prune_single_child_containers {
                    if let Some(only_child) = container.only_child() {
                        log::trace!("Simplify: collapsing single-child container tile");
                        return SimplifyAction::Replace(only_child);
                    }
                }
            }
        }

        self.tiles.insert(it, tile);
        SimplifyAction::Keep
    }

    pub(super) fn make_all_panes_children_of_tabs(&mut self, parent_is_tabs: bool, it: TileId) {
        let Some(mut tile) = self.tiles.remove(&it) else {
            log::warn!("Failed to find tile {it:?} during make_all_panes_children_of_tabs");
            return;
        };

        match &mut tile {
            Tile::Pane(_) => {
                if !parent_is_tabs {
                    // Add tabs to this pane:
                    log::trace!("Auto-adding Tabs-parent to pane {it:?}");
                    let new_id = self.insert_new(tile);
                    self.tiles
                        .insert(it, Tile::Container(Container::new_tabs(vec![new_id])));
                    return;
                }
            }
            Tile::Container(container) => {
                let is_tabs = container.kind() == ContainerKind::Tabs;
                for &child in container.children() {
                    self.make_all_panes_children_of_tabs(is_tabs, child);
                }
            }
        }

        self.tiles.insert(it, tile);
    }

    /// Returns true if the active tile was found in this tree.
    pub(super) fn make_active(
        &mut self,
        it: TileId,
        should_activate: &mut dyn FnMut(&Tile<Pane>) -> bool,
    ) -> bool {
        let Some(mut tile) = self.tiles.remove(&it) else {
            log::warn!("Failed to find tile {it:?} during make_active");
            return false;
        };

        let mut activate = should_activate(&tile);

        if let Tile::Container(container) = &mut tile {
            let mut active_child = None;
            for &child in container.children() {
                if self.make_active(child, should_activate) {
                    active_child = Some(child);
                }
            }

            if let Some(active_child) = active_child {
                if let Container::Tabs(tabs) = container {
                    tabs.set_active(active_child);
                }
            }

            activate |= active_child.is_some();
        }

        self.tiles.insert(it, tile);
        activate
    }
}

impl<Pane: PartialEq> Tiles<Pane> {
    /// Find the tile with the given pane.
    pub fn find_pane(&self, needle: &Pane) -> Option<TileId> {
        self.tiles
            .iter()
            .find(|(_, tile)| {
                if let Tile::Pane(pane) = *tile {
                    pane == needle
                } else {
                    false
                }
            })
            .map(|(tile_id, _)| *tile_id)
    }
}
