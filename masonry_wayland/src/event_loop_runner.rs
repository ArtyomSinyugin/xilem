use std::{
    collections::{HashMap, HashSet, VecDeque},
    sync::{
        Arc,
        mpsc::{Receiver, Sender},
    },
    time::Instant,
};

use accesskit::ActionRequest;
use accesskit_unix::Adapter;
use copypasta::{ClipboardContext, ClipboardProvider};
use dpi::PhysicalSize;
use masonry_core::{
    app::{RenderRoot, RenderRootOptions, RenderRootSignal, WindowSizePolicy},
    core::{
        DefaultProperties, ErasedAction, KeyboardEvent, NewWidget, PointerEvent, TextEvent, Widget,
        WidgetId, WindowEvent,
        keyboard::{Key, KeyState},
    },
    kurbo::Affine,
    peniko::Color,
    vello::{
        AaConfig, AaSupport, RenderParams, Renderer, RendererOptions, Scene,
        wgpu::{self},
    },
};
use smithay_winit::{
    ApplicationHandler, LoopHandler, WindowAttributes, WindowCore, WindowId as HandleId,
    WindowsRegistry, event_loop::WlEventLoop,
};
use tracing::{debug, info};

use crate::{
    AppDriver,
    app_driver::{DriverCtx, WindowId},
    convert_wayland_types::{logical_to_physical_rounded, masonry_resize_direction_to_wayland},
    vello_util::{RenderContext, RenderSurface},
};

/// A container for a window yet to be created.
///
/// This is stored inside [`MasonryState`] and will be created during the `resumed` event.
pub struct NewWindow {
    /// The id is set by the App, and can be created using the [`WindowId::next()`] method.
    ///
    /// Once the window is created, it can be accessed using this `id` through the
    /// [`DriverCtx::window()`] method.
    pub id: WindowId,
    /// Window attributes for the winit's [`Window`].
    ///
    /// A default attribute can be created using [`Window::default_attributes()`].
    ///
    /// [`Window`]: crate::winit::window::Window
    /// [`Window::default_attributes()`]: crate::winit::window::Window::default_attributes()
    pub attributes: WindowAttributes,
    /// The widget which will take up the entire contents of the new window.
    pub root_widget: NewWidget<dyn Widget>,
    /// The base color of the window.
    pub base_color: Color,
}

impl NewWindow {
    /// Create a new window with an automatically assigned [`WindowId`].
    ///
    /// See the documentation on the fields of this type for details of the parameters.
    pub fn new(attributes: WindowAttributes, root_widget: NewWidget<dyn Widget + 'static>) -> Self {
        Self::new_with_id(WindowId::next(), attributes, root_widget)
    }

    /// Create a new window with a custom assigned [`WindowId`].
    ///
    /// Use this when you need to specify a unique ID for the window, for example,
    /// for external tracking or state management.
    pub fn new_with_id(
        id: WindowId,
        attributes: WindowAttributes,
        root_widget: NewWidget<dyn Widget + 'static>,
    ) -> Self {
        Self {
            id,
            attributes,
            root_widget,
            base_color: Color::BLACK,
        }
    }

    /// Set the base color of the new window.
    ///
    /// The base color is the color of the background which all widgets in the window draw on top of.
    /// Masonry's current default theme assumes that this will be a very dark color for sufficient contrast.
    /// This is most useful for apps which want to for example support light mode.
    ///
    /// Please note that it is not currently supported to modify this once the app is running.
    /// This is not a fundamental limitation, and is only due to missing api design.
    pub fn with_base_color(mut self, base_color: Color) -> Self {
        self.base_color = base_color;
        self
    }
}
/// Per-Window state
pub struct Window {
    pub(crate) window_id: WindowId,
    pub(crate) render_root: RenderRoot,
    pub(crate) base_color: Color,
    pub(crate) scale_factor: f64,
}

impl Window {
    pub(crate) fn new(
        window_id: WindowId,
        root_widget: NewWidget<dyn Widget>,
        signal_sender: Sender<(WindowId, RenderRootSignal)>,
        default_properties: Arc<DefaultProperties>,
        base_color: Color,
        size: PhysicalSize<u32>,
        scale_factor: f64,
    ) -> Self {
        Self {
            window_id,
            render_root: RenderRoot::new(
                root_widget,
                move |signal| {
                    signal_sender.clone().send((window_id, signal)).unwrap();
                },
                RenderRootOptions {
                    default_properties,
                    use_system_fonts: true,
                    size_policy: WindowSizePolicy::User,
                    size,
                    scale_factor,
                    test_font: None,
                },
            ),
            base_color,
            scale_factor,
        }
    }

    /// Access the [`RenderRoot`] of this window.
    pub fn render_root(&mut self) -> &mut RenderRoot {
        &mut self.render_root
    }

    /// Access base color of this window.
    pub fn base_color(&mut self) -> &mut Color {
        &mut self.base_color
    }
}

// --- MARK: RUN
// TODO: somehow we need to run with default windows without attr
pub fn run(
    new_windows: Vec<NewWindow>,
    app_driver: impl AppDriver + 'static,
    default_properties: DefaultProperties,
) -> Result<(), String> {
    // If there is no default tracing subscriber, we set our own. If one has
    // already been set, we get an error which we swallow.
    // By now, we're about to take control of the event loop. The user is unlikely
    // to try to set their own subscriber once the event loop has started.
    let _ = masonry_core::app::try_init_tracing();

    let mut main_state = MainState {
        masonry_state: MasonryState::new(default_properties),
        app_driver: Box::new(app_driver),
    };

    let mut event_loop = WlEventLoop::init();

    //We shoud create windows after event loop is initialized. Otherwise trait method will return an error
    main_state.masonry_state.new_windows(new_windows)?;
    event_loop.run(&mut main_state)
}

/// User events for the masonry event loop
#[allow(dead_code)] // Variants are part of public API, may not be used internally
#[derive(Debug)]
pub enum MasonryUserEvent {
    // TODO: A more considered design here
    Action(WindowId, ErasedAction, WidgetId),
}

struct MainState<'a> {
    masonry_state: MasonryState<'a>,
    app_driver: Box<dyn AppDriver>,
}

pub struct MasonryState<'a> {
    pub(crate) render_cx: RenderContext,
    pub renderer: Option<Renderer>,
    #[cfg(feature = "tracy")]
    frame: Option<tracing_tracy::client::Frame>,
    surfaces: HashMap<HandleId, RenderSurface<'a>>,
    pub windows: HashMap<HandleId, Window>,
    window_id_to_handle_id: HashMap<WindowId, HandleId>,
    // Is `Some` if the most recently displayed frame was an animation frame.
    pub last_anim: Option<Instant>,
    pub signal_receiver: Receiver<(WindowId, RenderRootSignal)>,
    pub signal_sender: Sender<(WindowId, RenderRootSignal)>,
    pub default_properties: Arc<DefaultProperties>,
    pub clipboard_cx: ClipboardContext,
    pub(crate) new_windows: VecDeque<NewWindow>,
}

impl LoopHandler for MasonryState<'_> {}

impl MasonryState<'_> {
    pub fn new(default_properties: DefaultProperties) -> Self {
        let render_cx = RenderContext::new();

        let (signal_sender, signal_receiver) =
            std::sync::mpsc::channel::<(WindowId, RenderRootSignal)>();

        MasonryState {
            render_cx,
            renderer: None,
            surfaces: HashMap::new(),
            windows: HashMap::new(),
            window_id_to_handle_id: HashMap::new(),
            last_anim: None,
            #[cfg(feature = "tracy")]
            frame: None,
            signal_receiver,
            signal_sender,
            default_properties: Arc::new(default_properties),
            clipboard_cx: ClipboardContext::new().unwrap(),
            new_windows: VecDeque::new(),
        }
    }

    // --- MARK: RENDER
    fn render(
        surface: &mut RenderSurface<'_>,
        window: &mut Window,
        scale_factor: f64,
        scene: Scene,
        render_cx: &RenderContext,
        renderer: &mut Option<Renderer>,
    ) {
        let size = window.render_root.size();

        let transformed_scene = if scale_factor == 1.0 {
            None
        } else {
            let mut new_scene = Scene::new();
            new_scene.append(&scene, Some(Affine::scale(scale_factor)));
            Some(new_scene)
        };
        let scene_ref = transformed_scene.as_ref().unwrap_or(&scene);

        let dev_id = surface.dev_id;
        let device = &render_cx.devices[dev_id].device;
        let queue = &render_cx.devices[dev_id].queue;
        let renderer_options = RendererOptions {
            antialiasing_support: AaSupport::area_only(),
            ..Default::default()
        };
        let render_params = RenderParams {
            base_color: window.base_color,
            width: size.width,
            height: size.height,
            antialiasing_method: AaConfig::Area,
        };

        let _render_span = tracing::info_span!("Rendering using Vello").entered();
        renderer
            .get_or_insert_with(|| {
                #[cfg_attr(not(feature = "tracy"), expect(unused_mut, reason = "cfg"))]
                let mut renderer = Renderer::new(device, renderer_options).unwrap();
                #[cfg(feature = "tracy")]
                {
                    let new_profiler = wgpu_profiler::GpuProfiler::new_with_tracy_client(
                        wgpu_profiler::GpuProfilerSettings::default(),
                        // We don't have access to the adapter until we get  https://github.com/linebender/vello/pull/634
                        // Luckily, this `backend` is only used for visual display in the profiling, so we can just guess here
                        wgpu::Backend::Vulkan,
                        device,
                        queue,
                    )
                    .unwrap_or(renderer.profiler);
                    renderer.profiler = new_profiler;
                }
                renderer
            })
            .render_to_texture(
                device,
                queue,
                scene_ref,
                &surface.target_view,
                &render_params,
            )
            .expect("failed to render to surface");

        let Ok(surface_texture) = surface.surface.get_current_texture() else {
            tracing::error!("failed to acquire next swapchain texture");
            return;
        };

        // Copy the new surface content to the surface.
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("Surface Blit"),
        });
        surface.blitter.copy(
            device,
            &mut encoder,
            &surface.target_view,
            &surface_texture
                .texture
                .create_view(&wgpu::TextureViewDescriptor::default()),
        );
        queue.submit([encoder.finish()]);
        // TODO: do we need it
        // handle.pre_present_notify();
        surface_texture.present();
        {
            let _render_poll_span =
                tracing::info_span!("Waiting for GPU to finish rendering").entered();
            device.poll(wgpu::PollType::Wait).unwrap();
        }
    }

    pub(crate) fn new_windows(&mut self, mut new_windows: Vec<NewWindow>) -> Result<(), String> {
        for window in new_windows.drain(..) {
            let id: WindowId = window.id;
            match self.request_new_window(window.attributes.clone()) {
                Ok(_) => self.new_windows.push_back(window),
                Err(err) => {
                    tracing::error!("Failed to create window with id: {:?}\n{}", id, err);
                }
            }
        }
        Ok(())
    }

    pub fn close_window(&mut self, window_id: WindowId) {
        tracing::debug!(window_id = window_id.trace(), "closing window");
        let window_id = self
            .window_id_to_handle_id
            .remove(&window_id)
            .unwrap_or_else(|| panic!("could not found find window for id {window_id:?}"));
        self.surfaces.remove(&window_id);
        let _window = self.windows.remove(&window_id).unwrap();
    }

    fn handle_id(&self, window_id: WindowId) -> HandleId {
        self.window_id_to_handle_id
            .get(&window_id)
            .unwrap_or_else(|| panic!("could not find window for id {window_id:?}"))
            .to_owned()
    }

    pub(crate) fn window_mut(&mut self, window_id: WindowId) -> &mut Window {
        let handle_id = self.handle_id(window_id);
        self.windows.get_mut(&handle_id).unwrap()
    }

    fn handle_signals(&mut self, windows: &mut WindowsRegistry, app_driver: &mut dyn AppDriver) {
        let mut need_redraw = HashSet::<HandleId>::new();
        while let Some((window_id, signal)) = self.signal_receiver.try_iter().next() {
            let handle_id = self.handle_id(window_id);
            let window = self.windows.get_mut(&handle_id).unwrap();
            let handle = windows.get_mut(&handle_id).unwrap();
            match signal {
                RenderRootSignal::Action(action, widget_id) => {
                    debug!(
                        "Action {:?} on widget {:?}",
                        (*action).type_name(),
                        widget_id
                    );
                    app_driver.on_action(window_id, &mut DriverCtx::new(self), widget_id, action);
                }
                RenderRootSignal::StartIme => {
                    // handle.set_ime_allowed(true);
                }
                RenderRootSignal::EndIme => {
                    // handle.set_ime_allowed(false);
                }
                RenderRootSignal::ImeMoved(_position, _size) => {
                    // handle.set_ime_cursor_area(position, size);
                }
                RenderRootSignal::ClipboardStore(text) => {
                    self.clipboard_cx.set_contents(text).unwrap();
                }
                RenderRootSignal::RequestRedraw => {
                    need_redraw.insert(handle_id);
                }
                RenderRootSignal::RequestAnimFrame => {
                    // TODO
                    need_redraw.insert(handle_id);
                }
                RenderRootSignal::TakeFocus => {
                    // Wayland unsuported
                }
                RenderRootSignal::SetCursor(cursor) => {
                    handle.set_cursor(cursor);
                }
                RenderRootSignal::SetSize(size) => {
                    // TODO - Handle return value?
                    let _ = handle.request_inner_size(size);
                }
                RenderRootSignal::SetTitle(title) => {
                    handle.set_title(title);
                }
                RenderRootSignal::DragWindow => {
                    // TODO - Handle return value?
                    let _ = handle.drag_window();
                }
                RenderRootSignal::DragResizeWindow(direction) => {
                    // TODO - Handle return value?
                    let direction = masonry_resize_direction_to_wayland(direction);
                    let _ = handle.drag_resize_window(direction);
                }
                RenderRootSignal::ToggleMaximized => {
                    handle.set_maximized(!handle.is_maximized());
                }
                RenderRootSignal::Minimize => {
                    handle.set_minimized();
                }
                RenderRootSignal::Exit => {
                    self.stop();
                }
                RenderRootSignal::ShowWindowMenu(position) => {
                    handle.show_window_menu(position);
                }
                RenderRootSignal::WidgetSelectedInInspector(widget_id) => {
                    let Some(widget) = window.render_root.get_widget(widget_id) else {
                        return;
                    };
                    let widget_name = widget.short_type_name();
                    let display_name = if let Some(debug_text) = widget.get_debug_text() {
                        format!("{widget_name}<{debug_text}>")
                    } else {
                        widget_name.into()
                    };
                    info!("Widget selected in inspector: {widget_id} - {display_name}");
                }
            }
        }

        // If we're processing a lot of actions, we may have a lot of pending redraws.
        // We batch them up to avoid redundant requests.
        for id in &need_redraw {
            if let Some(window) = windows.get(id) {
                window.redraw_request();
            }
        }
    }

    fn handle_locked_signals(
        &mut self,
        windows: &mut WindowsRegistry,
        app_driver: &mut dyn AppDriver,
    ) {
        let mut need_redraw = HashSet::<HandleId>::new();
        while let Some((window_id, signal)) = self.signal_receiver.try_iter().next() {
            let handle_id = self.handle_id(window_id);
            let window = self.windows.get_mut(&handle_id).unwrap();
            let handle = windows.get_locked_mut(&handle_id).unwrap();
            match signal {
                RenderRootSignal::Action(action, widget_id) => {
                    debug!(
                        "Action {:?} on widget {:?}",
                        (*action).type_name(),
                        widget_id
                    );
                    app_driver.on_action(window_id, &mut DriverCtx::new(self), widget_id, action);
                }
                RenderRootSignal::StartIme => {
                    // handle.set_ime_allowed(true);
                }
                RenderRootSignal::EndIme => {
                    // handle.set_ime_allowed(false);
                }
                RenderRootSignal::ImeMoved(_position, _size) => {
                    // handle.set_ime_cursor_area(position, size);
                }
                RenderRootSignal::ClipboardStore(text) => {
                    self.clipboard_cx.set_contents(text).unwrap();
                }
                RenderRootSignal::RequestRedraw => {
                    need_redraw.insert(handle_id);
                }
                RenderRootSignal::RequestAnimFrame => {
                    // TODO
                    need_redraw.insert(handle_id);
                }
                RenderRootSignal::SetCursor(cursor) => {
                    handle.set_cursor(cursor);
                }
                RenderRootSignal::Exit => {
                    self.stop();
                }
                RenderRootSignal::WidgetSelectedInInspector(widget_id) => {
                    let Some(widget) = window.render_root.get_widget(widget_id) else {
                        return;
                    };
                    let widget_name = widget.short_type_name();
                    let display_name = if let Some(debug_text) = widget.get_debug_text() {
                        format!("{widget_name}<{debug_text}>")
                    } else {
                        widget_name.into()
                    };
                    info!("Widget selected in inspector: {widget_id} - {display_name}");
                }
                _ => {}
            }
        }

        // If we're processing a lot of actions, we may have a lot of pending redraws.
        // We batch them up to avoid redundant requests.
        for id in &need_redraw {
            if let Some(screenlock) = windows.get_locked(id) {
                screenlock.redraw_request();
            }
        }
    }

    fn draw(&mut self, handle: Arc<WindowCore>, adapter: &mut Adapter) {
        let id = handle.get_id();
        if let Some(window) = self.windows.get_mut(&id) {
            let size = window.render_root.size();
            if size.width == 0 || size.height == 0 {
                // Surface can't have a dimension of zero, remove the stale surface to save memory.
                self.surfaces.remove(&id);
                return;
            }
            // Get the existing surface or create a new one
            let surface = if let Some(surface) = self.surfaces.get_mut(&id) {
                // The window might have been resized, make sure the surface dimensions match.
                if surface.config.width != size.width || surface.config.height != size.height {
                    self.render_cx
                        .resize_surface(surface, size.width, size.height);
                }
                surface
            } else {
                let surface = create_surface(&mut self.render_cx, handle.clone(), size);
                self.surfaces.insert(id.clone(), surface);
                self.surfaces.get_mut(&id).unwrap()
            };

            let now = Instant::now();
            // TODO: this calculation uses wall-clock time of the paint call, which
            // potentially has jitter.
            //
            // See https://github.com/linebender/druid/issues/85 for discussion.
            let last = self.last_anim.take();
            let elapsed = last.map(|t| now.duration_since(t)).unwrap_or_default();

            window
                .render_root
                .handle_window_event(WindowEvent::AnimFrame(elapsed));

            // If this animation will continue, store the time.
            // If a new animation starts, then it will have zero reported elapsed time.
            let animation_continues = window.render_root.needs_anim();
            self.last_anim = animation_continues.then_some(now);

            let (scene, tree_update) = window.render_root.redraw();
            Self::render(
                surface,
                window,
                window.scale_factor,
                scene,
                &self.render_cx,
                &mut self.renderer,
            );
            #[cfg(feature = "tracy")]
            drop(self.frame.take());
            if let Some(tree_update) = tree_update {
                adapter.update_if_active(|| tree_update);
            }
        }
    }

    pub fn set_present_mode(&mut self, window_id: WindowId, present_mode: wgpu::PresentMode) {
        let handle_id = self.handle_id(window_id);
        let surface = self.surfaces.get_mut(&handle_id).unwrap();
        self.render_cx.set_present_mode(surface, present_mode);
    }
}

impl ApplicationHandler<MasonryUserEvent> for MainState<'_> {
    fn create_window(&mut self, new_window: Arc<WindowCore>) {
        let Some(window) = self.masonry_state.new_windows.pop_front() else {
            return;
        };
        let _ = self
            .masonry_state
            .window_id_to_handle_id
            .insert(window.id, new_window.get_id());
        // TODO: move this check to modification of base_color once winit exposes window transparency state
        if !window.attributes.transparent && window.base_color.components[3] != 1. {
            tracing::warn!(
                window_id = ?window.id,
                "New window with non-opaque base color doesn't support transparency - \
                you should call `.with_transparent(true)` on the new window's `WindowAttributes`."
            );
        }
        let scale_factor = self.masonry_state.default_scale_factor() as f64;
        let size = window
            .attributes
            .surface_size
            .map(|s| s.to_logical(scale_factor))
            .unwrap_or(self.masonry_state.default_window_size().clone());
        if self
            .masonry_state
            .windows
            .insert(
                new_window.get_id(),
                Window::new(
                    window.id,
                    window.root_widget,
                    self.masonry_state.signal_sender.clone(),
                    self.masonry_state.default_properties.clone(),
                    window.base_color,
                    logical_to_physical_rounded(size, scale_factor),
                    scale_factor,
                ),
            )
            .is_some()
        {
            panic!(
                "attempted to create a window with id {:?} but a window with that id already exists",
                window.id
            );
        }
        tracing::debug!(window_id = window.id.trace(), "creating window");
    }

    fn draw_handle(&mut self, window: Arc<WindowCore>, adapter: &mut Adapter) {
        self.masonry_state.draw(window, adapter);
    }

    fn keyboard_handle(&mut self, window: &HandleId, keyboard_event: KeyboardEvent) {
        if let Some(window) = self.masonry_state.windows.get_mut(&window) {
            // TODO - Detect in Masonry code instead
            let action_mod = keyboard_event.modifiers.ctrl();
            if let Key::Character(c) = &keyboard_event.key
                && c.as_str().eq_ignore_ascii_case("v")
                && action_mod
                && keyboard_event.state == KeyState::Down
            {
                window
                    .render_root
                    .handle_text_event(TextEvent::ClipboardPaste(
                        self.masonry_state.clipboard_cx.get_contents().unwrap(),
                    ));
            } else {
                window
                    .render_root
                    .handle_text_event(TextEvent::Keyboard(keyboard_event));
            }
        }
    }

    fn pointer_handle(&mut self, window: &HandleId, pointer_event: PointerEvent) {
        if let Some(window) = self.masonry_state.windows.get_mut(&window) {
            window.render_root.handle_pointer_event(pointer_event);
        }
    }

    fn rescale_handle(&mut self, window: &HandleId, scale_factor: f64) {
        if let Some(window) = self.masonry_state.windows.get_mut(&window) {
            window.scale_factor = scale_factor;
            window
                .render_root
                .handle_window_event(WindowEvent::Rescale(scale_factor));
        }
    }

    fn resize_handle(&mut self, window: &HandleId, size: PhysicalSize<u32>) {
        if let Some(window) = self.masonry_state.windows.get_mut(&window) {
            window
                .render_root
                .handle_window_event(WindowEvent::Resize(size));
        }
    }

    fn focus_handle(&mut self, window: &HandleId, new_focus: bool) {
        if let Some(window) = self.masonry_state.windows.get_mut(&window) {
            window
                .render_root
                .handle_text_event(TextEvent::WindowFocusChange(new_focus));
        }
    }

    fn accesskit_activate_handle(&self, _window: HandleId, _: &mut Adapter) {}
    fn accesskit_action_handle(
        &self,
        _window: HandleId,
        _action: ActionRequest,
        _adapter: &mut Adapter,
    ) {
    }
    fn accesskit_deactivate_handle(&self, _window: HandleId, _: &mut Adapter) {}

    fn close_handle(&mut self, window_id: &HandleId) {
        self.masonry_state.surfaces.remove(&window_id);
        let window = self
            .masonry_state
            .windows
            .remove(&window_id)
            .unwrap_or_else(|| panic!("could not found find window for id {window_id:?}"));
        self.masonry_state
            .window_id_to_handle_id
            .remove(&window.window_id);
        if self.masonry_state.windows.is_empty() {
            // Do smth before main event loop will be stopped
        }
    }

    fn user_events_handle(&mut self, event: MasonryUserEvent) {
        match event {
            MasonryUserEvent::Action(window_id, action, widget_id) => {
                let handle_id = self.masonry_state.handle_id(window_id);
                if let Some(window) = self.masonry_state.windows.get_mut(&handle_id) {
                    window
                        .render_root
                        .emit_signal(RenderRootSignal::Action(action, widget_id))
                }
            }
        }
    }

    fn user_signals_handle(&mut self, windows: &mut WindowsRegistry) {
        let app_driver = self.app_driver.as_mut();
        if !self.masonry_state.is_locked() {
            self.masonry_state.handle_signals(windows, app_driver);
        } else {
            self.masonry_state
                .handle_locked_signals(windows, app_driver);
        }
    }

    fn create_screenlock(
        &mut self,
        _new_screenlock: std::sync::Weak<WindowCore>,
        _size: dpi::LogicalSize<u32>,
    ) {
        todo!()
    }
}

fn create_surface<'s>(
    render_cx: &mut RenderContext,
    handle: Arc<WindowCore>,
    size: PhysicalSize<u32>,
) -> RenderSurface<'s> {
    assert!(
        size.width != 0 && size.height != 0,
        "cannot create a surface with a width or height of zero"
    );
    pollster::block_on(render_cx.create_surface(
        handle,
        size.width,
        size.height,
        wgpu::PresentMode::AutoVsync,
    ))
    .unwrap()
}
