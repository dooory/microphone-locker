//! Thin `egui` (`eframe`) settings window wired to [`AppController`].

use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex, OnceLock};

use eframe::egui;

use crate::audio::AudioBackend;
use crate::config::LockPolicy;
use crate::controller::{AppCommand, AppController, AppSnapshot, MicrophoneSnapshot};
use crate::tray::{SystemTray, TrayAction, window_icon_data};

/// Runs the native Windows system tray + `egui` settings window event loop.
pub fn run_app<B: AudioBackend + 'static>(
    controller: Arc<Mutex<AppController<B>>>,
    open_window_on_startup: bool,
    egui_ctx_slot: Arc<OnceLock<egui::Context>>,
) -> Result<(), Box<dyn std::error::Error>> {
    let viewport = egui::ViewportBuilder::default()
        .with_title("Microphone Locker")
        .with_inner_size([460.0, 440.0])
        .with_min_inner_size([360.0, 300.0])
        .with_visible(open_window_on_startup)
        .with_icon(Arc::new(window_icon_data()));

    let native_options = eframe::NativeOptions {
        viewport,
        run_and_return: true,
        ..Default::default()
    };

    eframe::run_native(
        "Microphone Locker",
        native_options,
        Box::new(move |cc| {
            let _ = egui_ctx_slot.set(cc.egui_ctx.clone());
            let (tray, tray_rx) = SystemTray::build(cc.egui_ctx.clone())?;
            Ok(Box::new(SettingsApp {
                controller,
                _tray: tray,
                tray_rx,
                hide_on_first_frame: !open_window_on_startup,
                quitting: false,
                last_error: None,
            }))
        }),
    )?;

    Ok(())
}

struct SettingsApp<B: AudioBackend> {
    controller: Arc<Mutex<AppController<B>>>,
    _tray: SystemTray,
    tray_rx: Receiver<TrayAction>,
    hide_on_first_frame: bool,
    quitting: bool,
    last_error: Option<String>,
}

impl<B: AudioBackend> SettingsApp<B> {
    fn snapshot(&self) -> Option<AppSnapshot> {
        self.controller.lock().ok().map(|ctrl| ctrl.snapshot())
    }

    fn send_command(&mut self, command: AppCommand) {
        match self.controller.lock() {
            Ok(mut ctrl) => match ctrl.handle_command(command) {
                Ok(()) => self.last_error = None,
                Err(err) => self.last_error = Some(err.to_string()),
            },
            Err(_) => {
                self.last_error = Some("Application controller lock was poisoned.".to_string());
            }
        }
    }

    fn drain_tray_actions(&mut self, ctx: &egui::Context) {
        while let Ok(action) = self.tray_rx.try_recv() {
            match action {
                TrayAction::ShowWindow => {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                    ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
                    ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                }
                TrayAction::Quit => {
                    self.quitting = true;
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            }
        }
    }
}

impl<B: AudioBackend> eframe::App for SettingsApp<B> {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if self.hide_on_first_frame {
            self.hide_on_first_frame = false;
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
        }

        self.drain_tray_actions(ctx);

        if ctx.input(|i| i.viewport().close_requested()) && !self.quitting {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ui, |ui| {
            ui.heading("Microphone Locker");
            ui.label(
                egui::RichText::new(
                    "Enforces Target Volumes on locked Microphones automatically.",
                )
                .weak(),
            );
            ui.add_space(10.0);

            if let Some(err) = self.last_error.clone() {
                egui::Frame::group(ui.style()).show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.colored_label(egui::Color32::LIGHT_RED, format!("Error: {err}"));
                        if ui.small_button("Dismiss").clicked() {
                            self.last_error = None;
                        }
                    });
                });
                ui.add_space(8.0);
            }

            let Some(mut snapshot) = self.snapshot() else {
                ui.colored_label(
                    egui::Color32::LIGHT_RED,
                    "Unable to read controller state.",
                );
                return;
            };

            // Global Default Volume control.
            egui::Frame::group(ui.style()).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.strong("Default Volume");
                ui.label(
                    egui::RichText::new(
                        "Applied immediately to all active Microphones following Default.",
                    )
                    .small()
                    .weak(),
                );
                ui.add_space(6.0);

                let mut default_volume = snapshot.default_volume;
                let slider = egui::Slider::new(&mut default_volume, 0..=100)
                    .suffix("%")
                    .integer();
                ui.spacing_mut().slider_width = (ui.available_width() - 80.0).max(120.0);
                if ui.add(slider).changed() && default_volume != snapshot.default_volume {
                    self.send_command(AppCommand::SetDefaultVolume(default_volume));
                    if let Some(updated) = self.snapshot() {
                        snapshot = updated;
                    }
                }
            });

            ui.add_space(12.0);
            ui.strong(format!(
                "Active Microphones ({})",
                snapshot.active_microphones.len()
            ));
            ui.add_space(6.0);

            let footer_height = 24.0;
            let list_height = (ui.available_height() - footer_height).max(80.0);

            egui::ScrollArea::vertical()
                .max_height(list_height)
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    if snapshot.active_microphones.is_empty() {
                        ui.label(
                            egui::RichText::new("No active Microphones detected.").weak(),
                        );
                    } else {
                        for mic in &snapshot.active_microphones {
                            render_microphone_row(ui, mic);
                            ui.add_space(6.0);
                        }
                    }
                });

            ui.add_space(4.0);
            ui.label(
                egui::RichText::new(
                    "Closing this window minimizes Microphone Locker to the system tray.",
                )
                .small()
                .weak(),
            );
        });
    }
}

fn render_microphone_row(ui: &mut egui::Ui, mic: &MicrophoneSnapshot) {
    let current_percent = (mic.current_volume_scalar.clamp(0.0, 1.0) * 100.0).round() as u8;

    egui::Frame::group(ui.style()).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            ui.strong(&mic.friendly_name);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let target_label = match mic.effective_target_volume {
                    Some(target) => format!("Target: {target}%"),
                    None => "Ignored".to_string(),
                };
                ui.strong(target_label);
            });
        });

        ui.horizontal(|ui| {
            let policy_text = match mic.lock_policy {
                LockPolicy::FollowDefault => "Following Default",
                LockPolicy::Custom(_) => "Custom Target Volume",
                LockPolicy::Ignored => "Ignored (not enforced)",
            };
            ui.label(egui::RichText::new(policy_text).small().weak());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(
                    egui::RichText::new(format!("Current: {current_percent}%"))
                        .small()
                        .weak(),
                );
            });
        });

        ui.add_space(4.0);
        ui.add(
            egui::ProgressBar::new(mic.current_volume_scalar.clamp(0.0, 1.0))
                .desired_height(6.0),
        );
    });
}
