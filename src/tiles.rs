use std::collections::{HashMap, HashSet};

use egui::{Pos2, Rect, Ui};

use super::{
    Behavior, Container, DropContext, GcAction, Grid, InsertionPoint, Layout, LayoutInsertion,
    Linear, LinearDir, SimplificationOptions, SimplifyAction, Tabs, Tile, TileId, UiResponse,
};

/// Contains all tile state, but no root.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Tiles<Pane> {
    pub tiles: HashMap<TileId, Tile<Pane>>,

    /// Filled in by the layout step at the start of each frame.
    #[serde(default, skip)]
    pub(super) rects: HashMap<TileId, Rect>,
}

impl<Pane> Default for Tiles<Pane> {
    fn default() -> Self {
        Self {
            tiles: Default::default(),
            rects: Default::default(),
        }
    }
}

// ----------------------------------------------------------------------------

impl<Pane> Tiles<Pane> {
    pub(super) fn try_rect(&self, tile_id: TileId) -> Option<Rect> {
        self.rects.get(&tile_id).copied()
    }

    pub(super) fn rect(&self, tile_id: TileId) -> Rect {
        let rect = self.try_rect(tile_id);
        debug_assert!(rect.is_some(), "Failed to find rect for {tile_id:?}");
        rect.unwrap_or(egui::Rect::from_min_max(Pos2::ZERO, Pos2::ZERO))
    }

    pub fn get(&self, tile_id: TileId) -> Option<&Tile<Pane>> {
        self.tiles.get(&tile_id)
    }

    pub fn get_mut(&mut self, tile_id: TileId) -> Option<&mut Tile<Pane>> {
        self.tiles.get_mut(&tile_id)
    }

    #[must_use]
    pub fn insert_tile(&mut self, tile: Tile<Pane>) -> TileId {
        let id = TileId::random();
        self.tiles.insert(id, tile);
        id
    }

    #[must_use]
    pub fn insert_pane(&mut self, pane: Pane) -> TileId {
        self.insert_tile(Tile::Pane(pane))
    }

    #[must_use]
    pub fn insert_container(&mut self, contaioner: Container) -> TileId {
        self.insert_tile(Tile::Container(contaioner))
    }

    #[must_use]
    pub fn insert_tab_tile(&mut self, children: Vec<TileId>) -> TileId {
        self.insert_tile(Tile::Container(Container::new_tabs(children)))
    }

    #[must_use]
    pub fn insert_horizontal_tile(&mut self, children: Vec<TileId>) -> TileId {
        self.insert_tile(Tile::Container(Container::new_linear(
            LinearDir::Horizontal,
            children,
        )))
    }

    #[must_use]
    pub fn insert_vertical_tile(&mut self, children: Vec<TileId>) -> TileId {
        self.insert_tile(Tile::Container(Container::new_linear(
            LinearDir::Vertical,
            children,
        )))
    }

    #[must_use]
    pub fn insert_grid_tile(&mut self, children: Vec<TileId>) -> TileId {
        self.insert_tile(Tile::Container(Container::new_grid(children)))
    }

    pub(super) fn insert(&mut self, insertion_point: InsertionPoint, child_id: TileId) {
        let InsertionPoint {
            parent_id,
            insertion,
        } = insertion_point;

        let Some(mut tile) = self.tiles.remove(&parent_id) else {
            log::warn!("Failed to insert: could not find parent {parent_id:?}");
            return;
        };

        match insertion {
            LayoutInsertion::Tabs(index) => {
                if let Tile::Container(Container::Tabs(tabs)) = &mut tile {
                    let index = index.min(tabs.children.len());
                    tabs.children.insert(index, child_id);
                    tabs.active = child_id;
                    self.tiles.insert(parent_id, tile);
                } else {
                    let new_tile_id = self.insert_tile(tile);
                    let mut tabs = Tabs::new(vec![new_tile_id]);
                    tabs.children.insert(index.min(1), child_id);
                    tabs.active = child_id;
                    self.tiles
                        .insert(parent_id, Tile::Container(Container::Tabs(tabs)));
                }
            }
            LayoutInsertion::Horizontal(index) => {
                if let Tile::Container(Container::Linear(Linear {
                    dir: LinearDir::Horizontal,
                    children,
                    ..
                })) = &mut tile
                {
                    let index = index.min(children.len());
                    children.insert(index, child_id);
                    self.tiles.insert(parent_id, tile);
                } else {
                    let new_tile_id = self.insert_tile(tile);
                    let mut linear = Linear::new(LinearDir::Horizontal, vec![new_tile_id]);
                    linear.children.insert(index.min(1), child_id);
                    self.tiles
                        .insert(parent_id, Tile::Container(Container::Linear(linear)));
                }
            }
            LayoutInsertion::Vertical(index) => {
                if let Tile::Container(Container::Linear(Linear {
                    dir: LinearDir::Vertical,
                    children,
                    ..
                })) = &mut tile
                {
                    let index = index.min(children.len());
                    children.insert(index, child_id);
                    self.tiles.insert(parent_id, tile);
                } else {
                    let new_tile_id = self.insert_tile(tile);
                    let mut linear = Linear::new(LinearDir::Vertical, vec![new_tile_id]);
                    linear.children.insert(index.min(1), child_id);
                    self.tiles
                        .insert(parent_id, Tile::Container(Container::Linear(linear)));
                }
            }
            LayoutInsertion::Grid(insert_location) => {
                if let Tile::Container(Container::Grid(grid)) = &mut tile {
                    grid.locations.retain(|_, pos| *pos != insert_location);
                    grid.locations.insert(child_id, insert_location);
                    grid.children.push(child_id);
                    self.tiles.insert(parent_id, tile);
                } else {
                    let new_tile_id = self.insert_tile(tile);
                    let mut grid = Grid::new(vec![new_tile_id, child_id]);
                    grid.locations.insert(child_id, insert_location);
                    self.tiles
                        .insert(parent_id, Tile::Container(Container::Grid(grid)));
                }
            }
        }
    }

    pub(super) fn gc_root(&mut self, behavior: &mut dyn Behavior<Pane>, root_id: TileId) {
        let mut visited = HashSet::default();
        self.gc_tile_id(behavior, &mut visited, root_id);

        if visited.len() < self.tiles.len() {
            log::warn!(
                "GC collecting tiles: {:?}",
                self.tiles
                    .keys()
                    .filter(|id| !visited.contains(id))
                    .collect::<Vec<_>>()
            );
        }

        self.tiles.retain(|tile_id, _| visited.contains(tile_id));
    }

    fn gc_tile_id(
        &mut self,
        behavior: &mut dyn Behavior<Pane>,
        visited: &mut HashSet<TileId>,
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
            container.layout_recursive(self, style, behavior, rect);
        }

        self.tiles.insert(tile_id, tile);
    }

    pub(super) fn tile_ui(
        &mut self,
        behavior: &mut dyn Behavior<Pane>,
        drop_context: &mut DropContext,
        ui: &mut Ui,
        tile_id: TileId,
    ) {
        // NOTE: important that we get the rect and tile in two steps,
        // otherwise we could loose the tile when there is no rect.
        let Some(rect) = self.try_rect(tile_id) else {
            log::warn!("Failed to find rect for tile {tile_id:?} during ui");
            return
        };
        let Some(mut tile) = self.tiles.remove(&tile_id) else {
            log::warn!("Failed to find tile {tile_id:?} during ui");
            return
        };

        let drop_context_was_enabled = drop_context.enabled;
        if Some(tile_id) == drop_context.dragged_tile_id {
            // Can't drag a tile onto self or any children
            drop_context.enabled = false;
        }
        drop_context.on_tile(behavior, ui.style(), tile_id, rect, &tile);

        // Each tile gets its own `Ui`, nested inside each other, with proper clip rectangles.
        let mut ui = egui::Ui::new(
            ui.ctx().clone(),
            ui.layer_id(),
            ui.id().with(tile_id),
            rect,
            rect,
        );
        match &mut tile {
            Tile::Pane(pane) => {
                if behavior.pane_ui(&mut ui, tile_id, pane) == UiResponse::DragStarted {
                    ui.memory_mut(|mem| mem.set_dragged_id(tile_id.id()));
                }
            }
            Tile::Container(container) => {
                container.ui(self, behavior, drop_context, &mut ui, rect, tile_id);
            }
        };

        self.tiles.insert(tile_id, tile);
        drop_context.enabled = drop_context_was_enabled;
    }

    pub(super) fn simplify(
        &mut self,
        options: &SimplificationOptions,
        it: TileId,
    ) -> SimplifyAction {
        let Some(mut tile) = self.tiles.remove(&it) else {
            log::warn!("Failed to find tile {it:?} during simplify");
            return SimplifyAction::Remove;
        };

        if let Tile::Container(container) = &mut tile {
            // TODO(emilk): join nested versions of the same horizontal/vertical layouts

            container.simplify_children(|child| self.simplify(options, child));

            if container.layout() == Layout::Tabs {
                if options.prune_empty_tabs && container.is_empty() {
                    log::debug!("Simplify: removing empty tabs tile");
                    return SimplifyAction::Remove;
                }
                if options.prune_single_child_tabs && container.children().len() == 1 {
                    if options.all_panes_must_have_tabs
                        && matches!(self.get(container.children()[0]), Some(Tile::Pane(_)))
                    {
                        // Keep it
                    } else {
                        log::debug!("Simplify: collapsing single-child tabs tile");
                        return SimplifyAction::Replace(container.children()[0]);
                    }
                }
            } else {
                if options.prune_empty_layouts && container.is_empty() {
                    log::debug!("Simplify: removing empty layout tile");
                    return SimplifyAction::Remove;
                }
                if options.prune_single_child_layouts && container.children().len() == 1 {
                    log::debug!("Simplify: collapsing single-child layout tile");
                    return SimplifyAction::Replace(container.children()[0]);
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
                    log::debug!("Auto-adding Tabs-parent to pane {it:?}");
                    let new_id = TileId::random();
                    self.tiles.insert(new_id, tile);
                    self.tiles
                        .insert(it, Tile::Container(Container::new_tabs(vec![new_id])));
                    return;
                }
            }
            Tile::Container(container) => {
                let is_tabs = container.layout() == Layout::Tabs;
                for &child in container.children() {
                    self.make_all_panes_children_of_tabs(is_tabs, child);
                }
            }
        }

        self.tiles.insert(it, tile);
    }
}