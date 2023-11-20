use egui::{NumExt as _, Pos2, Rect, Ui, ViewportBuilder, ViewportId};

use crate::{ContainerKind, UiResponse};

use super::{
    is_possible_drag, Behavior, Container, DropContext, InsertionPoint, SimplificationOptions,
    SimplifyAction, Tile, TileId, Tiles,
};

/// Tile that has its own egui viewport (native window).
#[derive(Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
pub struct ViewportTile {
    pub root: TileId,

    /// monitor-space position of the window.
    pub screen_pos: Pos2,

    pub dragged: bool,
}

/// The top level type. Contains all persistent state, including layouts and sizes.
///
/// You'll usually construct this once and then store it, calling [`Tree::ui`] each frame.
///
/// See [the crate-level documentation](crate) for a complete example.
///
/// ## How to construct a [`Tree`]
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
#[derive(Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
pub struct Tree<Pane> {
    /// None = empty tree
    pub root: Option<TileId>,

    /// All the tiles in the tree.
    pub tiles: Tiles<Pane>,

    /// Tiles that have their own viewports (native windows).
    /// They form their own roots.
    pub viewport_tiles: Vec<ViewportTile>,
}

impl<Pane> Default for Tree<Pane> {
    // An empty tree
    fn default() -> Self {
        Self {
            root: None,
            tiles: Default::default(),
            viewport_tiles: Default::default(),
        }
    }
}

impl<Pane: std::fmt::Debug> std::fmt::Debug for Tree<Pane> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Print a hierarchical view of the tree:
        fn format_tile<Pane: std::fmt::Debug>(
            f: &mut std::fmt::Formatter<'_>,
            tiles: &Tiles<Pane>,
            indent: usize,
            tile_id: TileId,
        ) -> std::fmt::Result {
            write!(f, "{} {tile_id:?}: ", "  ".repeat(indent))?;
            if let Some(tile) = tiles.get(tile_id) {
                match tile {
                    Tile::Pane(pane) => writeln!(f, "Pane {pane:?}"),
                    Tile::Container(container) => {
                        writeln!(
                            f,
                            "{}",
                            match container {
                                Container::Tabs(_) => "Tabs",
                                Container::Linear(_) => "Linear",
                                Container::Grid(_) => "Grid",
                            }
                        )?;
                        for &child in container.children() {
                            format_tile(f, tiles, indent + 1, child)?;
                        }
                        Ok(())
                    }
                }
            } else {
                writeln!(f, "DANGLING")
            }
        }

        if let Some(root) = self.root {
            writeln!(f, "Tree {{")?;
            format_tile(f, &self.tiles, 1, root)?;
            write!(f, "}}")
        } else {
            writeln!(f, "Tree {{ }}")
        }
    }
}

// ----------------------------------------------------------------------------

impl<Pane> Tree<Pane> {
    pub fn empty() -> Self {
        Self::default()
    }

    /// The most flexible constructor, allowing you to set up the tiles
    /// however you want.
    pub fn new(root: TileId, tiles: Tiles<Pane>) -> Self {
        Self {
            root: Some(root),
            tiles,
            viewport_tiles: Default::default(),
        }
    }

    /// Create a top-level [`crate::Tabs`] container with the given panes.
    pub fn new_tabs(panes: Vec<Pane>) -> Self {
        Self::new_container(ContainerKind::Tabs, panes)
    }

    /// Create a top-level horizontal [`crate::Linear`] container with the given panes.
    pub fn new_horizontal(panes: Vec<Pane>) -> Self {
        Self::new_container(ContainerKind::Horizontal, panes)
    }

    /// Create a top-level vertical [`crate::Linear`] container with the given panes.
    pub fn new_vertical(panes: Vec<Pane>) -> Self {
        Self::new_container(ContainerKind::Vertical, panes)
    }

    /// Create a top-level [`crate::Grid`] container with the given panes.
    pub fn new_grid(panes: Vec<Pane>) -> Self {
        Self::new_container(ContainerKind::Grid, panes)
    }

    /// Create a top-level container with the given panes.
    pub fn new_container(kind: ContainerKind, panes: Vec<Pane>) -> Self {
        let mut tiles = Tiles::default();
        let tile_ids = panes
            .into_iter()
            .map(|pane| tiles.insert_pane(pane))
            .collect();
        let root = tiles.insert_new(Tile::Container(Container::new(kind, tile_ids)));
        Self::new(root, tiles)
    }

    /// Check if [`Self::root`] is [`None`].
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.root.is_none() && self.viewport_tiles.is_empty()
    }

    #[inline]
    pub fn root(&self) -> Option<TileId> {
        self.root
    }

    #[inline]
    pub fn roots(&self) -> Vec<TileId> {
        let mut roots = self
            .viewport_tiles
            .iter()
            .map(|t| t.root)
            .collect::<Vec<_>>();
        if let Some(root) = self.root {
            roots.push(root);
        }
        roots
    }

    pub fn is_root(&self, tile: TileId) -> bool {
        self.root == Some(tile)
    }

    /// Tiles are visible by default.
    ///
    /// Invisible tiles still retain their place in the tile hierarchy.
    pub fn is_visible(&self, tile_id: TileId) -> bool {
        self.tiles.is_visible(tile_id)
    }

    /// Tiles are visible by default.
    ///
    /// Invisible tiles still retain their place in the tile hierarchy.
    pub fn set_visible(&mut self, tile_id: TileId, visible: bool) {
        self.tiles.set_visible(tile_id, visible);
    }

    /// Show the tree in the given [`Ui`].
    ///
    /// The tree will use upp all the available space - nothing more, nothing less.
    pub fn ui(&mut self, behavior: &mut dyn Behavior<Pane>, ui: &mut Ui) {
        self.simplify(&behavior.simplification_options());

        self.gc(behavior);

        self.tiles.rects.clear();

        // Check if anything is being dragged:
        let mut drop_context = DropContext {
            enabled: true,
            dragged_tile_id: self.dragged_id(ui.ctx()),
            mouse_pos: ui.input(|i| i.pointer.hover_pos()),
            best_dist_sq: f32::INFINITY,
            best_insertion: None,
            preview_rect: None,
        };

        if let Some(root) = self.root {
            self.tiles
                .layout_tile(ui.style(), behavior, ui.available_rect_before_wrap(), root);

            self.tile_ui(behavior, &mut drop_context, ui, root);
        }

        for ViewportTile {
            root,
            screen_pos,
            mut dragged,
        } in std::mem::take(&mut self.viewport_tiles)
        {
            let viewport_id = ViewportId::from_hash_of(root);
            let title = behavior.tab_title_for_tile(&self.tiles, root);

            if dragged {
                ui.ctx()
                    .send_viewport_cmd_to(viewport_id, egui::ViewportCommand::StartDrag);
            }

            let close_requested = ui.ctx().show_viewport_immediate(
                viewport_id,
                ViewportBuilder::default()
                    .with_title(title.text())
                    .with_position(screen_pos)
                    .with_inner_size([480.0, 320.0]),
                |ctx, _class| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        self.tiles.layout_tile(
                            ui.style(),
                            behavior,
                            ui.available_rect_before_wrap(),
                            root,
                        );
                        self.tile_ui(behavior, &mut drop_context, ui, root);
                    });

                    dragged &= !ui.input(|i| i.pointer.any_released());

                    ctx.input(|i| i.viewport().close_requested())
                },
            );

            dragged &= !ui.input(|i| i.pointer.any_released());

            if close_requested {
                // Drop the whole tree
            } else {
                self.viewport_tiles.push(ViewportTile {
                    root,
                    screen_pos,
                    dragged,
                });
            }
        }

        self.preview_dragged_tile(behavior, &drop_context, ui);
    }

    pub(super) fn tile_ui(
        &mut self,
        behavior: &mut dyn Behavior<Pane>,
        drop_context: &mut DropContext,
        ui: &mut Ui,
        tile_id: TileId,
    ) {
        if !self.is_visible(tile_id) {
            return;
        }
        // NOTE: important that we get the rect and tile in two steps,
        // otherwise we could loose the tile when there is no rect.
        let Some(rect) = self.tiles.try_rect(tile_id) else {
            log::warn!("Failed to find rect for tile {tile_id:?} during ui");
            return
        };
        let Some(mut tile) = self.tiles.remove(tile_id) else {
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
                    ui.memory_mut(|mem| mem.set_dragged_id(tile_id.egui_id()));
                }
            }
            Tile::Container(container) => {
                container.ui(self, behavior, drop_context, &mut ui, rect, tile_id);
            }
        };

        self.tiles.insert(tile_id, tile);
        drop_context.enabled = drop_context_was_enabled;
    }

    /// Recursively "activate" the ancestors of the tiles that matches the given predicate.
    ///
    /// This means making the matching tiles and its ancestors the active tab in any tab layout.
    ///
    /// Returns `true` if a tab was made active.
    pub fn make_active(&mut self, mut should_activate: impl FnMut(&Tile<Pane>) -> bool) -> bool {
        if let Some(root) = self.root {
            self.tiles.make_active(root, &mut should_activate)
        } else {
            false
        }
    }

    fn preview_dragged_tile(
        &mut self,
        behavior: &mut dyn Behavior<Pane>,
        drop_context: &DropContext,
        ui: &mut Ui,
    ) {
        let (Some(mouse_pos), Some(dragged_tile_id)) =
            (drop_context.mouse_pos, drop_context.dragged_tile_id) else { return; };

        // Preview what is being dragged:
        let preview_size = egui::vec2(300.0, 200.0);
        let preview_id = egui::Id::new((dragged_tile_id, "preview"));
        if ui.ctx().screen_rect().contains(mouse_pos) {
            ui.output_mut(|o| o.cursor_icon = egui::CursorIcon::Grabbing);
            egui::Area::new(preview_id)
                .pivot(egui::Align2::CENTER_CENTER)
                .current_pos(mouse_pos)
                .interactable(false)
                .show(ui.ctx(), |ui| {
                    // ui.set_min_size(preview_size);
                    // ui.set_max_size(preview_size);
                    behavior.drag_ui(&self.tiles, ui, dragged_tile_id);
                });
        } else {
            let screen_pos = if let Some(inner_rect) = ui.ctx().input(|i| i.viewport().inner_rect) {
                mouse_pos + inner_rect.min.to_vec2() + egui::vec2(-0.5 * preview_size.x, -16.0)
            } else {
                mouse_pos
            };

            // Spawn the new viewport tile right away, to get a nice preview for it:
            ui.memory_mut(|mem| mem.stop_dragging());

            behavior.on_edit();
            self.remove_tile_id_from_parent(dragged_tile_id);
            self.viewport_tiles.push(ViewportTile {
                root: dragged_tile_id,
                screen_pos,
                dragged: true,
            });
        }

        if let Some(preview_rect) = drop_context.preview_rect {
            let preview_rect = smooth_preview_rect(ui.ctx(), dragged_tile_id, preview_rect);

            let parent_rect = drop_context
                .best_insertion
                .and_then(|insertion_point| self.tiles.try_rect(insertion_point.parent_id));

            behavior.paint_drag_preview(ui.visuals(), ui.painter(), parent_rect, preview_rect);

            if behavior.preview_dragged_panes() {
                // TODO(emilk): add support for previewing containers too.
                if preview_rect.width() > 32.0 && preview_rect.height() > 32.0 {
                    if let Some(Tile::Pane(pane)) = self.tiles.get_mut(dragged_tile_id) {
                        let _ = behavior.pane_ui(
                            &mut ui.child_ui(preview_rect, *ui.layout()),
                            dragged_tile_id,
                            pane,
                        );
                    }
                }
            }
        }

        if ui.input(|i| i.pointer.any_released()) {
            ui.memory_mut(|mem| mem.stop_dragging());

            if let Some(insertion_point) = drop_context.best_insertion {
                behavior.on_edit();
                self.move_tile(dragged_tile_id, insertion_point);
            }

            clear_smooth_preview_rect(ui.ctx(), dragged_tile_id);
        }
    }

    /// Simplify and normalize the tree using the given options.
    ///
    /// This is also called at the start of [`Self::ui`].
    pub fn simplify(&mut self, options: &SimplificationOptions) {
        for root in self.roots() {
            match self.tiles.simplify(options, root, None) {
                SimplifyAction::Keep => {}
                SimplifyAction::Remove => {
                    self.root = None;
                }
                SimplifyAction::Replace(new_root) => {
                    self.root = Some(new_root);
                }
            }
        }

        if options.all_panes_must_have_tabs {
            for root in self.roots() {
                self.tiles.make_all_panes_children_of_tabs(false, root);
            }
        }
    }

    /// Garbage-collect tiles that are no longer reachable from the root tile.
    ///
    /// This is also called by [`Self::ui`], so usually you don't need to call this yourself.
    pub fn gc(&mut self, behavior: &mut dyn Behavior<Pane>) {
        self.tiles.gc_roots(behavior, &self.roots());
    }

    /// Move the given tile to the given insertion point.
    pub(super) fn move_tile(&mut self, moved_tile_id: TileId, insertion_point: InsertionPoint) {
        log::trace!(
            "Moving {moved_tile_id:?} into {:?}",
            insertion_point.insertion
        );

        if let Some((prev_parent_id, source_index)) = self.remove_tile_id_from_parent(moved_tile_id)
        {
            // Check to see if we are moving a tile within the same container:

            if prev_parent_id == insertion_point.parent_id {
                let parent_tile = self.tiles.get_mut(prev_parent_id);

                if let Some(Tile::Container(container)) = parent_tile {
                    if container.kind() == insertion_point.insertion.kind() {
                        let dest_index = insertion_point.insertion.index();
                        log::trace!(
                            "Moving within the same parent: {source_index} -> {dest_index}"
                        );
                        // lets swap the two indices

                        let adjusted_index = if source_index < dest_index {
                            // We removed an earlier element, so we need to adjust the index:
                            dest_index - 1
                        } else {
                            dest_index
                        };

                        match container {
                            Container::Tabs(tabs) => {
                                let insertion_index = adjusted_index.min(tabs.children.len());
                                tabs.children.insert(insertion_index, moved_tile_id);
                                tabs.active = Some(moved_tile_id);
                            }
                            Container::Linear(linear) => {
                                let insertion_index = adjusted_index.min(linear.children.len());
                                linear.children.insert(insertion_index, moved_tile_id);
                            }
                            Container::Grid(grid) => {
                                // the grid allow holes in its children list, so don't use `adjusted_index`
                                let dest_tile = grid.replace_at(dest_index, moved_tile_id);
                                if let Some(dest) = dest_tile {
                                    grid.insert_at(source_index, dest);
                                }
                            }
                        }
                        return; // done
                    }
                }
            }
        }

        // Moving to a new parent
        self.tiles.insert_at(insertion_point, moved_tile_id);
    }

    /// Find the currently dragged tile, if any.
    pub fn dragged_id(&self, ctx: &egui::Context) -> Option<TileId> {
        if !is_possible_drag(ctx) {
            // We're not sure we're dragging _at all_ yet.
            return None;
        }

        for tile_id in self.tiles.tile_ids() {
            if self.is_root(tile_id) {
                continue; // not allowed to drag root
            }

            let id = tile_id.egui_id();
            let is_tile_being_dragged = ctx.memory(|mem| mem.is_being_dragged(id));
            if is_tile_being_dragged {
                // Abort drags on escape:
                if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
                    ctx.memory_mut(|mem| mem.stop_dragging());
                    return None;
                }

                return Some(tile_id);
            }
        }
        None
    }

    /// This removes the given tile from the parents list of children.
    ///
    /// The [`Tile`] itself is not removed from [`Self::tiles`].
    ///
    /// Performs no simplifcations.
    ///
    /// If found, the parent tile and the child's index is returned.
    pub(super) fn remove_tile_id_from_parent(
        &mut self,
        remove_me: TileId,
    ) -> Option<(TileId, usize)> {
        for (parent_id, parent) in self.tiles.iter_mut() {
            if let Tile::Container(container) = parent {
                if let Some(child_index) = container.remove_child(remove_me) {
                    return Some((*parent_id, child_index));
                }
            }
        }
        None
    }
}

// ----------------------------------------------------------------------------

/// We store the preview rect in egui temp storage so that it is not serialized,
/// and so that a user could re-create the [`Tree`] each frame and still get smooth previews.
fn smooth_preview_rect_id(dragged_tile_id: TileId) -> egui::Id {
    egui::Id::new((dragged_tile_id, "smoothed_preview_rect"))
}

fn clear_smooth_preview_rect(ctx: &egui::Context, dragged_tile_id: TileId) {
    let data_id = smooth_preview_rect_id(dragged_tile_id);
    ctx.data_mut(|data| data.remove::<Rect>(data_id));
}

/// Take the preview rectangle and smooth it over time.
fn smooth_preview_rect(ctx: &egui::Context, dragged_tile_id: TileId, new_rect: Rect) -> Rect {
    let data_id = smooth_preview_rect_id(dragged_tile_id);

    let dt = ctx.input(|input| input.stable_dt).at_most(0.1);

    let mut requires_repaint = false;

    let smoothed = ctx.data_mut(|data| {
        let smoothed: &mut Rect = data.get_temp_mut_or(data_id, new_rect);

        let t = egui::emath::exponential_smooth_factor(0.9, 0.05, dt);

        *smoothed = smoothed.lerp_towards(&new_rect, t);

        let diff = smoothed.min.distance(new_rect.min) + smoothed.max.distance(new_rect.max);
        if diff < 0.5 {
            *smoothed = new_rect;
        } else {
            requires_repaint = true;
        }
        *smoothed
    });

    if requires_repaint {
        ctx.request_repaint();
    }

    smoothed
}
