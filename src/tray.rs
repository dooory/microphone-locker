//! Windows system tray icon, context menu, and programmatic icon rendering.

use std::sync::mpsc::{self, Receiver};

use eframe::egui;
use tray_icon::menu::{Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};

pub const ICON_SIZE: u32 = 32;

/// User actions triggered from the Windows system tray icon or its context menu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayAction {
    ShowWindow,
    TogglePaused,
    Quit,
}

/// Owns the live Windows system tray icon; dropping it removes the icon from the tray.
pub struct SystemTray {
    tray_icon: TrayIcon,
    pause_item: MenuItem,
    active_icon: Icon,
    paused_icon: Icon,
    last_paused: bool,
}

impl SystemTray {
    /// Creates the Windows system tray icon and wires left-click and context menu
    /// events to wake `ctx` and deliver [`TrayAction`]s over the returned receiver.
    ///
    /// Must be called on the UI / Win32 message-loop thread.
    pub fn build(
        ctx: egui::Context,
    ) -> Result<(Self, Receiver<TrayAction>), Box<dyn std::error::Error + Send + Sync>> {
        let show_item = MenuItem::new("Show Window", true, None);
        let pause_item = MenuItem::new("Pause Locking", true, None);
        let quit_item = MenuItem::new("Quit", true, None);
        let show_id: MenuId = show_item.id().clone();
        let pause_id: MenuId = pause_item.id().clone();
        let quit_id: MenuId = quit_item.id().clone();

        let menu = Menu::new();
        menu.append(&show_item)?;
        menu.append(&pause_item)?;
        menu.append(&PredefinedMenuItem::separator())?;
        menu.append(&quit_item)?;

        let active_icon = Icon::from_rgba(render_microphone_icon_rgba(), ICON_SIZE, ICON_SIZE)?;
        let paused_icon = Icon::from_rgba(render_paused_icon_rgba(), ICON_SIZE, ICON_SIZE)?;

        let tray_icon = TrayIconBuilder::new()
            .with_tooltip("Microphone Locker")
            .with_icon(active_icon.clone())
            .with_menu(Box::new(menu))
            .with_menu_on_left_click(false)
            .build()?;

        let (action_tx, action_rx) = mpsc::channel();

        let tray_tx = action_tx.clone();
        let tray_ctx = ctx.clone();
        TrayIconEvent::set_event_handler(Some(move |event| {
            if matches!(
                event,
                TrayIconEvent::Click {
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Up,
                    ..
                } | TrayIconEvent::DoubleClick {
                    button: MouseButton::Left,
                    ..
                }
            ) {
                let _ = tray_tx.send(TrayAction::ShowWindow);
                tray_ctx.request_repaint();
            }
        }));

        MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
            if event.id == show_id {
                let _ = action_tx.send(TrayAction::ShowWindow);
                ctx.request_repaint();
            } else if event.id == pause_id {
                let _ = action_tx.send(TrayAction::TogglePaused);
                ctx.request_repaint();
            } else if event.id == quit_id {
                let _ = action_tx.send(TrayAction::Quit);
                ctx.request_repaint();
            }
        }));

        Ok((
            Self {
                tray_icon,
                pause_item,
                active_icon,
                paused_icon,
                last_paused: false,
            },
            action_rx,
        ))
    }

    /// Synchronizes the tray icon, tooltip, and pause menu item with the global Paused state.
    pub fn sync_paused(&mut self, paused: bool) {
        if self.last_paused == paused {
            return;
        }
        self.last_paused = paused;

        let icon = if paused {
            self.paused_icon.clone()
        } else {
            self.active_icon.clone()
        };
        let _ = self.tray_icon.set_icon(Some(icon));

        let tooltip = if paused {
            "Microphone Locker (Paused)"
        } else {
            "Microphone Locker"
        };
        let _ = self.tray_icon.set_tooltip(Some(tooltip));

        self.pause_item.set_text(if paused {
            "Resume Locking"
        } else {
            "Pause Locking"
        });
    }
}

/// Returns window icon data matching the system tray icon.
pub fn window_icon_data() -> egui::IconData {
    egui::IconData {
        rgba: render_microphone_icon_rgba(),
        width: ICON_SIZE,
        height: ICON_SIZE,
    }
}

/// Generates a crisp 32x32 RGBA microphone badge icon in pure Rust.
pub fn render_microphone_icon_rgba() -> Vec<u8> {
    let size = ICON_SIZE as usize;
    let mut rgba = vec![0u8; size * size * 4];
    let center = (ICON_SIZE as f32 - 1.0) * 0.5;

    for y in 0..size {
        for x in 0..size {
            let px = x as f32;
            let py = y as f32;

            let p = [px, py];

            // Circular badge background (visible on both light and dark Windows taskbars).
            let bg_dist = ((px - center).hypot(py - center)) - 14.8;
            let bg_alpha = (0.5 - bg_dist).clamp(0.0, 1.0);

            // White microphone silhouette: capsule, U-cradle, stem, and base.
            let capsule_dist = segment_distance(p, [center, 9.5], [center, 14.5]) - 3.3;

            let arc_dx = px - center;
            let arc_dy = py - 14.5;
            let arc_dist = if arc_dy >= 0.0 {
                (arc_dx.hypot(arc_dy) - 6.0).abs() - 1.15
            } else {
                let left_tip = (px - (center - 6.0)).hypot(py - 14.5) - 1.15;
                let right_tip = (px - (center + 6.0)).hypot(py - 14.5) - 1.15;
                left_tip.min(right_tip)
            };

            let stem_dist = segment_distance(p, [center, 20.5], [center, 24.4]) - 1.05;
            let base_dist =
                segment_distance(p, [center - 3.8, 24.6], [center + 3.8, 24.6]) - 1.05;

            let fg_dist = capsule_dist.min(arc_dist).min(stem_dist).min(base_dist);
            let fg_alpha = (0.5 - fg_dist).clamp(0.0, 1.0);

            // Composite white foreground over teal-blue badge.
            let bg_r = 28.0_f32;
            let bg_g = 110.0_f32;
            let bg_b = 140.0_f32;

            let r = bg_r * (1.0 - fg_alpha) + 255.0 * fg_alpha;
            let g = bg_g * (1.0 - fg_alpha) + 255.0 * fg_alpha;
            let b = bg_b * (1.0 - fg_alpha) + 255.0 * fg_alpha;
            let a = bg_alpha * 255.0;

            let idx = (y * size + x) * 4;
            rgba[idx] = r.round() as u8;
            rgba[idx + 1] = g.round() as u8;
            rgba[idx + 2] = b.round() as u8;
            rgba[idx + 3] = a.round() as u8;
        }
    }

    rgba
}

/// Generates a crisp 32x32 RGBA Paused badge icon (amber circle with pause bars) in pure Rust.
pub fn render_paused_icon_rgba() -> Vec<u8> {
    let size = ICON_SIZE as usize;
    let mut rgba = vec![0u8; size * size * 4];
    let center = (ICON_SIZE as f32 - 1.0) * 0.5;

    for y in 0..size {
        for x in 0..size {
            let px = x as f32;
            let py = y as f32;
            let p = [px, py];

            // Circular badge background matching the active icon geometry.
            let bg_dist = ((px - center).hypot(py - center)) - 14.8;
            let bg_alpha = (0.5 - bg_dist).clamp(0.0, 1.0);

            // Two crisp vertical pause bars.
            let left_bar = segment_distance(p, [center - 4.2, 9.8], [center - 4.2, 22.2]) - 2.1;
            let right_bar = segment_distance(p, [center + 4.2, 9.8], [center + 4.2, 22.2]) - 2.1;
            let fg_dist = left_bar.min(right_bar);
            let fg_alpha = (0.5 - fg_dist).clamp(0.0, 1.0);

            // Composite white foreground over warm amber badge.
            let bg_r = 196.0_f32;
            let bg_g = 128.0_f32;
            let bg_b = 28.0_f32;

            let r = bg_r * (1.0 - fg_alpha) + 255.0 * fg_alpha;
            let g = bg_g * (1.0 - fg_alpha) + 255.0 * fg_alpha;
            let b = bg_b * (1.0 - fg_alpha) + 255.0 * fg_alpha;
            let a = bg_alpha * 255.0;

            let idx = (y * size + x) * 4;
            rgba[idx] = r.round() as u8;
            rgba[idx + 1] = g.round() as u8;
            rgba[idx + 2] = b.round() as u8;
            rgba[idx + 3] = a.round() as u8;
        }
    }

    rgba
}

fn segment_distance(p: [f32; 2], a: [f32; 2], b: [f32; 2]) -> f32 {
    let abx = b[0] - a[0];
    let aby = b[1] - a[1];
    let len_sq = abx * abx + aby * aby;
    let t = if len_sq > 0.0 {
        (((p[0] - a[0]) * abx + (p[1] - a[1]) * aby) / len_sq).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let cx = a[0] + t * abx;
    let cy = a[1] + t * aby;
    (p[0] - cx).hypot(p[1] - cy)
}

#[cfg(test)]
mod tests {
    use super::{ICON_SIZE, render_microphone_icon_rgba, render_paused_icon_rgba};

    #[test]
    fn active_and_paused_tray_icons_are_valid_and_visually_distinct() {
        let active = render_microphone_icon_rgba();
        let paused = render_paused_icon_rgba();
        let expected_len = (ICON_SIZE * ICON_SIZE * 4) as usize;

        assert_eq!(active.len(), expected_len);
        assert_eq!(paused.len(), expected_len);
        assert_ne!(active, paused);
    }
}
