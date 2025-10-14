// Copyright 2025 the Xilem Authors
// SPDX-License-Identifier: Apache-2.0

use dpi::{LogicalSize, PhysicalSize};
use masonry_core::core::ResizeDirection;
use smithay_winit::xdg::ResizeEdge;

pub(crate) fn masonry_resize_direction_to_wayland(dir: ResizeDirection) -> ResizeEdge {
    match dir {
        ResizeDirection::East => ResizeEdge::Right,
        ResizeDirection::North => ResizeEdge::Top,
        ResizeDirection::NorthEast => ResizeEdge::TopRight,
        ResizeDirection::NorthWest => ResizeEdge::TopLeft,
        ResizeDirection::South => ResizeEdge::Bottom,
        ResizeDirection::SouthEast => ResizeEdge::BottomRight,
        ResizeDirection::SouthWest => ResizeEdge::BottomLeft,
        ResizeDirection::West => ResizeEdge::Left,
    }
}

#[inline]
pub(crate) fn logical_to_physical_rounded(
    size: LogicalSize<u32>,
    scale_factor: f64,
) -> PhysicalSize<u32> {
    let width = size.width as f64 * scale_factor;
    let height = size.height as f64 * scale_factor;
    (width.round(), height.round()).into()
}
