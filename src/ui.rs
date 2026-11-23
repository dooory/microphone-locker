//! Thin `egui` (`eframe`) settings window wired to [`AppController`].

use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex, OnceLock};

use eframe::egui;

use crate::audio::AudioBackend;
use crate::config::LockPolicy;
use crate::controller::{
    AppCommand, AppController, AppSnapshot, DisconnectedMicrophoneSnapshot, MicrophoneSnapshot,
};
use crate::startup::StartupRegistry;
use crate::tray::{SystemTray, TrayAction, window_icon_data};

/// Runs the native Windows system tray + `egui` settings window event loop.
pub fn run_app<B: AudioBackend + 'static, S: StartupRegistry + 'static>(
    controller: Arc<Mutex<AppController<B, S>>>,
    open_window_on_startup: bool,
    egui_ctx_slot: Arc<OnceLock<egui::Context>>,
    show_window_rx: Receiver<()>,
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
                tray,
                tray_rx,
                show_window_rx,
                hide_on_first_frame: !open_window_on_startup,
                quitting: false,
                last_error: None,
            }))
        }),
    )?;

    Ok(())
}

struct SettingsApp<B: AudioBackend, S: StartupRegistry> {
    controller: Arc<Mutex<AppController<B, S>>>,
    tray: SystemTray,
    tray_rx: Receiver<TrayAction>,
    show_window_rx: Receiver<()>,
    hide_on_first_frame: bool,
    quitting: bool,
    last_error: Option<String>,
}

impl<B: AudioBackend, S: StartupRegistry> SettingsApp<B, S> {
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

    fn send_command_and_refresh(&mut self, command: AppCommand, snapshot: &mut AppSnapshot) {
        self.send_command(command);
        if let Some(updated) = self.snapshot() {
            *snapshot = updated;
        }
    }

    fn show_and_focus_window(ctx: &egui::Context) {
        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
        ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
        ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
    }

    fn drain_tray_actions(&mut self, ctx: &egui::Context) {
        while let Ok(action) = self.tray_rx.try_recv() {
            match action {
                TrayAction::ShowWindow => {
                    Self::show_and_focus_window(ctx);
                }
                TrayAction::TogglePaused => {
                    if let Some(snapshot) = self.snapshot() {
                        self.send_command(AppCommand::SetPaused(!snapshot.paused));
                    }
                }
                TrayAction::Quit => {
                    self.quitting = true;
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            }
        }
    }

    fn drain_show_window_signals(&mut self, ctx: &egui::Context) {
        let mut requested = false;
        while self.show_window_rx.try_recv().is_ok() {
            requested = true;
        }
        if requested {
            Self::show_and_focus_window(ctx);
        }
    }
}

impl<B: AudioBackend, S: StartupRegistry> eframe::App for SettingsApp<B, S> {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if self.hide_on_first_frame {
            self.hide_on_first_frame = false;
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
        }

        self.drain_tray_actions(ctx);
        self.drain_show_window_signals(ctx);

        if let Some(snapshot) = self.snapshot() {
            self.tray.sync_paused(snapshot.paused);
        }

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

            // Global temporary Paused toggle.
            egui::Frame::group(ui.style()).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        if snapshot.paused {
                            ui.colored_label(
                                egui::Color32::from_rgb(230, 155, 45),
                                egui::RichText::new("Paused").strong(),
                            );
                            ui.label(
                                egui::RichText::new(
                                    "Volume enforcement is temporarily suspended; Lock Policies are preserved.",
                                )
                                .small()
                                .weak(),
                            );
                        } else {
                            ui.colored_label(
                                egui::Color32::from_rgb(65, 175, 140),
                                egui::RichText::new("Active").strong(),
                            );
                            ui.label(
                                egui::RichText::new(
                                    "Enforcing Target Volumes on all locked Microphones.",
                                )
                                .small()
                                .weak(),
                            );
                        }
                    });

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let button_label = if snapshot.paused {
                            "Resume Locking"
                        } else {
                            "Pause Locking"
                        };
                        if ui.button(button_label).clicked() {
                            self.send_command_and_refresh(
                                AppCommand::SetPaused(!snapshot.paused),
                                &mut snapshot,
                            );
                            self.tray.sync_paused(snapshot.paused);
                        }
                    });
                });
            });

            ui.add_space(10.0);

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
                    self.send_command_and_refresh(
                        AppCommand::SetDefaultVolume(default_volume),
                        &mut snapshot,
                    );
                }
            });

            ui.add_space(8.0);
            let mut start_with_windows = snapshot.start_with_windows;
            if ui
                .checkbox(&mut start_with_windows, "Start with Windows")
                .changed()
                && start_with_windows != snapshot.start_with_windows
            {
                self.send_command_and_refresh(
                    AppCommand::SetStartWithWindows(start_with_windows),
                    &mut snapshot,
                );
            }

            ui.add_space(12.0);
            ui.strong(format!(
                "Active Microphones ({})",
                snapshot.active_microphones.len()
            ));
            ui.add_space(6.0);

            let footer_height = 24.0;
            let list_height = (ui.available_height() - footer_height).max(80.0);

            let mut pending_commands = Vec::new();

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
                            if let Some(cmd) =
                                render_microphone_row(ui, mic, snapshot.default_volume)
                            {
                                pending_commands.push(cmd);
                            }
                            ui.add_space(6.0);
                        }
                    }

                    if !snapshot.disconnected_microphones.is_empty() {
                        ui.add_space(10.0);
                        egui::CollapsingHeader::new(format!(
                            "Disconnected Microphones ({})",
                            snapshot.disconnected_microphones.len()
                        ))
                        .default_open(false)
                        .show(ui, |ui| {
                            ui.add_space(4.0);
                            for mic in &snapshot.disconnected_microphones {
                                if let Some(cmd) = render_disconnected_microphone_row(
                                    ui,
                                    mic,
                                    snapshot.default_volume,
                                ) {
                                    pending_commands.push(cmd);
                                }
                                ui.add_space(6.0);
                            }
                        });
                    }
                });

            for cmd in pending_commands {
                self.send_command(cmd);
            }

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

fn render_microphone_row(
    ui: &mut egui::Ui,
    mic: &MicrophoneSnapshot,
    default_volume: u8,
) -> Option<AppCommand> {
    let current_percent = (mic.current_volume_scalar.clamp(0.0, 1.0) * 100.0).round() as u8;
    let is_ignored = mic.lock_policy == LockPolicy::Ignored;
    let is_custom = matches!(mic.lock_policy, LockPolicy::Custom(_));
    let mut command = None;

    ui.push_id(&mic.endpoint_id, |ui| {
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.set_width(ui.available_width());

            ui.horizontal(|ui| {
                ui.strong(&mic.friendly_name);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let mut ignored = is_ignored;
                    if ui.checkbox(&mut ignored, "Ignored").changed() {
                        command = Some(AppCommand::SetMicrophoneIgnored {
                            endpoint_id: mic.endpoint_id.clone(),
                            ignored,
                        });
                    }
                });
            });

            ui.horizontal(|ui| {
                let policy_text = format_lock_policy_label(mic.lock_policy, default_volume);
                ui.label(egui::RichText::new(policy_text).small().weak());

                if is_custom && ui.small_button("Reset to Default").clicked() {
                    command = Some(AppCommand::SetMicrophoneLockPolicy {
                        endpoint_id: mic.endpoint_id.clone(),
                        lock_policy: LockPolicy::FollowDefault,
                    });
                }

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(
                        egui::RichText::new(format!("Current: {current_percent}%"))
                            .small()
                            .weak(),
                    );
                });
            });

            ui.add_space(4.0);

            let displayed_target = mic.effective_target_volume.unwrap_or(current_percent);
            let mut slider_value = displayed_target;
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("Target Volume").small());
                ui.spacing_mut().slider_width = (ui.available_width() - 80.0).max(120.0);
                let slider = egui::Slider::new(&mut slider_value, 0..=100)
                    .suffix("%")
                    .integer();
                if ui.add_enabled(!is_ignored, slider).changed()
                    && slider_value != displayed_target
                {
                    command = Some(AppCommand::SetMicrophoneLockPolicy {
                        endpoint_id: mic.endpoint_id.clone(),
                        lock_policy: LockPolicy::Custom(slider_value),
                    });
                }
            });
        });
    });

    command
}

fn render_disconnected_microphone_row(
    ui: &mut egui::Ui,
    mic: &DisconnectedMicrophoneSnapshot,
    default_volume: u8,
) -> Option<AppCommand> {
    let mut command = None;

    ui.push_id(("disconnected", &mic.endpoint_id), |ui| {
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.set_width(ui.available_width());

            ui.horizontal(|ui| {
                ui.strong(&mic.friendly_name);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.small_button("Forget").clicked() {
                        command = Some(AppCommand::ForgetMicrophone {
                            endpoint_id: mic.endpoint_id.clone(),
                        });
                    }
                });
            });

            let policy_text = format_lock_policy_label(mic.lock_policy, default_volume);
            ui.label(egui::RichText::new(policy_text).small().weak());
        });
    });

    command
}

fn format_lock_policy_label(lock_policy: LockPolicy, default_volume: u8) -> String {
    match lock_policy {
        LockPolicy::FollowDefault => format!("Following Default ({default_volume}%)"),
        LockPolicy::Custom(target) => format!("Custom Target Volume ({target}%)"),
        LockPolicy::Ignored => "Ignored (not enforced)".to_string(),
    }
}
