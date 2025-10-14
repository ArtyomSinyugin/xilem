// Copyright 2024 the Xilem Authors
// SPDX-License-Identifier: Apache-2.0

use masonry_core::app::RenderRoot;
use masonry_core::core::{ErasedAction, WidgetId};
use smithay_winit::{LoopHandler, WindowId};

use crate::event_loop_runner::Window;
use crate::{MasonryState, NewWindow};

/// Context for the [`AppDriver`] trait.
pub struct DriverCtx<'a, 's> {
    state: &'a mut MasonryState<'s>,
}

impl<'a, 's> DriverCtx<'a, 's> {
    pub(crate) fn new(state: &'a mut MasonryState<'s>) -> Self {
        Self { state }
    }
}

/// A trait for defining how your app interacts with the Masonry widget tree.
///
/// When launching your app with [`crate::app::run`], you need to provide
/// a type that implements this trait.
#[expect(unused_variables, reason = "Default impls doesn't use arguments")]
pub trait AppDriver {
    /// A hook which will be executed when a widget emits an `action`.
    ///
    /// This action is type-erased, and the type of action emitted will depend on.
    /// Each widget should document which types of action it might emit.
    fn on_action(
        &mut self,
        window_id: WindowId,
        ctx: &mut DriverCtx<'_, '_>,
        widget_id: WidgetId,
        action: ErasedAction,
    );

    /// A hook which will be executed when the application starts, to allow initial configuration of the `MasonryState`.
    ///
    /// Use cases include loading fonts.
    ///
    /// There are circumstances under which this will be called multiple times during the lifecycle of your app.
    /// This is not intended to be the behaviour of Masonry Winit long-term, but this method should currently
    /// not assume it will only be called once (but should feel free to waste work if it is called multiple times,
    /// for example, as the mentioned circumstances are very rare).
    // TODO: Turn into something like on window created, or split into two.
    fn on_start(&mut self, state: &mut MasonryState) {}

    /// A hook called when a user has requested to close a window.
    fn on_close_requested(&mut self, window_id: WindowId, ctx: &mut DriverCtx<'_, '_>) {
        ctx.exit();
    }
}

impl DriverCtx<'_, '_> {
    // TODO - Add method to create timer

    /// Access the [`RenderRoot`] of the given window.
    ///
    /// # Panics
    ///
    /// Panics if the window cannot be found.
    pub fn render_root(&mut self, window_id: WindowId) -> Option<&mut RenderRoot> {
        match self.state.windows.get_mut(&window_id) {
            Some(window) => Some(&mut window.render_root),
            None => None,
        }
    }

    /// Access the [`Window`] state of the given window.
    ///
    /// # Panics
    ///
    /// Panics if the window cannot be found.
    pub fn window(&mut self, window_id: WindowId) -> Option<&mut Window> {
        self.state.windows.get_mut(&window_id)
    }

    /// Creates a new window.
    ///
    /// # Panics
    ///
    /// Panics if the window id is already used by another window.
    pub fn create_window(&mut self, new_window: NewWindow) -> Result<(), String> {
        self.state.new_windows(vec![new_window])
    }

    /// Closes the given window.
    ///
    /// # Panics
    ///
    /// Panics if the window cannot be found.
    pub fn close_window(&mut self, window_id: WindowId) {
        let _w = self.state.windows.remove(&window_id).unwrap();
    }

    /// Exits the application (stops the event loop).
    pub fn exit(&self) {
        self.state.stop();
    }
}
