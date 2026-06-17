//! Turns the live [`Grid`](velo_de_core::Grid) into GLES render elements:
//! the Velo background, Overview tile backgrounds with an accent selection
//! halo, an accent border around the focused window, `wlr_layer_shell`
//! surfaces (e.g. a Velo-shell top bar or Velo Launcher), every visible
//! mapped surface placed via [`velo_de_core::place_window`], and a software
//! mouse cursor.

use smithay::backend::allocator::Fourcc;
use smithay::backend::renderer::element::memory::{MemoryRenderBuffer, MemoryRenderBufferRenderElement};
use smithay::backend::renderer::element::surface::{render_elements_from_surface_tree, WaylandSurfaceRenderElement};
use smithay::backend::renderer::element::Kind;
use smithay::backend::renderer::gles::GlesRenderer;
use smithay::backend::renderer::utils::draw_render_elements;
use smithay::backend::renderer::{Color32F, Frame, Renderer, RendererSuper};
use smithay::input::pointer::{CursorImageStatus, CursorImageSurfaceData};
use smithay::utils::{Logical, Physical, Point, Rectangle, Scale, Size as SmithaySize, Transform};
use smithay::wayland::compositor::with_states;
use smithay::wayland::shell::wlr_layer::{Layer, LayerSurface};

use velo_de_core::place_window;

use crate::state::State;

/// Render one frame into `framebuffer` and present it via `backend.submit`
/// (the caller does the actual `submit`/event-loop bookkeeping).
pub fn render_frame(
    state: &mut State,
    renderer: &mut GlesRenderer,
    framebuffer: &mut <GlesRenderer as RendererSuper>::Framebuffer<'_>,
    output_size: SmithaySize<i32, Physical>,
    transform: Transform,
) -> Result<(), Box<dyn std::error::Error>> {
    let theme = state.config.theme.clone();
    let viewport = state.grid.viewport();
    let focused = state.grid.focused_window();
    let border = theme.border_width.round().max(0.0) as i32;
    let damage = [Rectangle::from_size(output_size)];
    let usable_loc = state.usable_area().loc;
    let offset = Point::<i32, Physical>::from((usable_loc.x, usable_loc.y));

    let mut elements: Vec<WaylandSurfaceRenderElement<GlesRenderer>> = Vec::new();
    let mut tiles: Vec<(Rectangle<i32, Physical>, bool)> = Vec::new();
    let mut borders: Vec<Rectangle<i32, Physical>> = Vec::new();

    // Top/Overlay layer surfaces (e.g. a Velo Launcher popup) render above
    // everything else.
    for (surface, layer, geometry) in state.layer_entries() {
        if matches!(layer, Layer::Top | Layer::Overlay) {
            push_layer_elements(renderer, &mut elements, surface, *geometry);
        }
    }

    for frame in state.grid.frame() {
        let tile = Rectangle::new(
            Point::from((frame.rect.x.round() as i32 + offset.x, frame.rect.y.round() as i32 + offset.y)),
            SmithaySize::from((frame.rect.w.round() as i32, frame.rect.h.round() as i32)),
        );
        tiles.push((tile, frame.is_overview_selection));

        for w in &frame.windows {
            if !w.visible {
                continue;
            }
            let Some(surface) = state.toplevel_for(w.id) else { continue };
            let surface = surface.wl_surface().clone();

            let screen = place_window(frame.rect, viewport, w.rect);
            let loc = Point::<i32, Physical>::from((screen.x.round() as i32 + offset.x, screen.y.round() as i32 + offset.y));
            let size = SmithaySize::<i32, Physical>::from((screen.w.round() as i32, screen.h.round() as i32));
            let scale_x = frame.rect.w / viewport.w;
            let scale_y = frame.rect.h / viewport.h;

            if Some(w.id) == focused && frame.is_current {
                borders.push(Rectangle::new(
                    Point::from((loc.x - border, loc.y - border)),
                    SmithaySize::from((size.w + 2 * border, size.h + 2 * border)),
                ));
            }

            elements.extend(render_elements_from_surface_tree::<_, WaylandSurfaceRenderElement<GlesRenderer>>(
                renderer,
                &surface,
                loc,
                Scale::from((scale_x, scale_y)),
                1.0,
                Kind::Unspecified,
            ));
        }
    }

    // Background/Bottom layer surfaces (e.g. a Velo-shell top bar, a
    // wallpaper) render below the Spaces content.
    for (surface, layer, geometry) in state.layer_entries() {
        if matches!(layer, Layer::Background | Layer::Bottom) {
            push_layer_elements(renderer, &mut elements, surface, *geometry);
        }
    }

    // xdg-popups (menus, tooltips) render above their context. Collect at the
    // front of the element list so they sit on top of windows and layers.
    for popup in state.popup_surfaces() {
        if !popup.alive() {
            continue;
        }
        let screen = state.popup_screen_loc(popup);
        let loc = Point::<i32, Physical>::from((screen.x, screen.y));
        let popup_elements = render_elements_from_surface_tree::<_, WaylandSurfaceRenderElement<GlesRenderer>>(
            renderer,
            popup.wl_surface(),
            loc,
            Scale::from(1.0),
            1.0,
            Kind::Unspecified,
        );
        for (i, element) in popup_elements.into_iter().enumerate() {
            elements.insert(i, element);
        }
    }

    // Collect cursor surface elements now, while renderer is not yet borrowed
    // by gpu_frame. render_elements_from_surface_tree needs &mut renderer; once
    // gpu_frame exists that borrow is held and we can't call it again.
    let cursor_pos = state.cursor_pos;
    let cursor_status = state.cursor_status.clone();
    let mut cursor_elements: Vec<WaylandSurfaceRenderElement<GlesRenderer>> = Vec::new();
    if let CursorImageStatus::Surface(ref cursor_surface) = cursor_status {
        let hotspot = with_states(cursor_surface, |states| {
            states
                .data_map
                .get::<CursorImageSurfaceData>()
                .map(|d| d.lock().unwrap().hotspot)
                .unwrap_or_default()
        });
        let loc = Point::<i32, Physical>::from((
            cursor_pos.x as i32 - hotspot.x,
            cursor_pos.y as i32 - hotspot.y,
        ));
        cursor_elements.extend(render_elements_from_surface_tree::<_, WaylandSurfaceRenderElement<GlesRenderer>>(
            renderer,
            cursor_surface,
            loc,
            Scale::from(1.0),
            1.0,
            Kind::Unspecified,
        ));
    }

    // No client cursor surface: build a render element from the loaded xcursor
    // theme frame (or the built-in arrow fallback). Done before gpu_frame
    // borrows the renderer (MemoryRenderBufferRenderElement::from_buffer needs
    // &mut renderer for the texture upload).
    let mut cursor_memory_element: Option<MemoryRenderBufferRenderElement<GlesRenderer>> = None;
    if cursor_elements.is_empty() && !matches!(cursor_status, CursorImageStatus::Hidden) {
        let time = state.start_time.elapsed().as_millis() as u32;
        let frame = state.cursor_frames.frame_at(time);
        let buffer = MemoryRenderBuffer::from_slice(
            &frame.pixels,
            // pixels are byte order R,G,B,A -> Abgr8888 (see cursor.rs).
            Fourcc::Abgr8888,
            (frame.width as i32, frame.height as i32),
            1,
            Transform::Normal,
            None,
        );
        let loc = Point::<f64, Physical>::from((
            cursor_pos.x - frame.hotspot_x as f64,
            cursor_pos.y - frame.hotspot_y as f64,
        ));
        cursor_memory_element = MemoryRenderBufferRenderElement::from_buffer(renderer, loc, &buffer, None, None, None, Kind::Cursor).ok();
    }

    let mut gpu_frame = renderer.render(framebuffer, output_size, transform)?;
    gpu_frame.clear(Color32F::from(theme.background.to_array()), &damage)?;

    for (tile, is_selection) in &tiles {
        if *is_selection {
            let halo = Rectangle::new(
                Point::from((tile.loc.x - border, tile.loc.y - border)),
                SmithaySize::from((tile.size.w + 2 * border, tile.size.h + 2 * border)),
            );
            gpu_frame.draw_solid(halo, &damage, Color32F::from(theme.accent.to_array()))?;
        }
        gpu_frame.draw_solid(*tile, &damage, Color32F::from(theme.view_background.to_array()))?;
    }

    for rect in borders {
        gpu_frame.draw_solid(rect, &damage, Color32F::from(theme.accent.to_array()))?;
    }
    draw_render_elements(&mut gpu_frame, 1.0, &elements, &damage)?;

    // Draw the software cursor on top of everything else.
    if !cursor_elements.is_empty() {
        draw_render_elements(&mut gpu_frame, 1.0, &cursor_elements, &damage)?;
    } else if let Some(element) = cursor_memory_element {
        draw_render_elements(&mut gpu_frame, 1.0, std::slice::from_ref(&element), &damage)?;
    }

    let _ = gpu_frame.finish()?;

    Ok(())
}

/// Push render elements for a `wlr_layer_shell` surface placed at its
/// computed `geometry` (output-relative logical pixels, numerically equal to
/// physical pixels at the integer scale this backend always uses).
fn push_layer_elements(renderer: &mut GlesRenderer, elements: &mut Vec<WaylandSurfaceRenderElement<GlesRenderer>>, surface: &LayerSurface, geometry: Rectangle<i32, Logical>) {
    let loc = Point::<i32, Physical>::from((geometry.loc.x, geometry.loc.y));
    elements.extend(render_elements_from_surface_tree::<_, WaylandSurfaceRenderElement<GlesRenderer>>(
        renderer,
        surface.wl_surface(),
        loc,
        Scale::from(1.0),
        1.0,
        Kind::Unspecified,
    ));
}
