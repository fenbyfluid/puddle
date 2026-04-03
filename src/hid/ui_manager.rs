use crate::hid::messages::{
    DisplayContent, EncoderLabel, HardwareControl, HidInputEvent, InputReport, OutputReport, ScreenSpec, VariableEntry,
};
use anyhow::Result;
use log::trace;
use puddle::messages::{
    ClientMessage, CommandUpdate, CoreMessage, DriveState, MotionAction, MotionCommand, MotionCommandFields,
};
use puddle::units::{Acceleration, Position, Velocity};
use puddle::{ControllerId, CoreState, SystemLimits};
use std::collections::HashMap;
use std::ffi::CString;

// TODO: This is a very minimal first implementation
pub struct UiManager {
    io: super::IoManager,
    // TODO: We need to encapsulate an entire UI state with variables, not just the ScreenSpec
    active_screen: ScreenSpec,
    motion_commands: [MotionCommand; 2],
    last_sent_variables: HashMap<u8, VariableEntry>,
    holding_start: bool,
}

const DEFAULT_COMMANDS: [MotionCommand; 2] = [
    MotionCommand {
        position: Position::ZERO,
        velocity: Velocity::from_meters_per_second(1),
        acceleration: Acceleration::from_millimeters_per_second_squared(500),
        deceleration: Acceleration::from_millimeters_per_second_squared(500),
    },
    MotionCommand {
        position: Position::ZERO,
        velocity: Velocity::from_meters_per_second(1),
        acceleration: Acceleration::from_millimeters_per_second_squared(500),
        deceleration: Acceleration::from_millimeters_per_second_squared(500),
    },
];

impl UiManager {
    pub fn new(io: super::IoManager) -> Self {
        let default_screen = ScreenSpec {
            screen_id: 1,
            encoder_labels: [
                EncoderLabel { primary: c"{10}".to_owned(), secondary: c"{8}".to_owned() }, // Start, Enable / Disable / Ack.
                EncoderLabel { primary: c"{11}".to_owned(), secondary: c"{9}".to_owned() }, // End / Stroke, Resume / Pause
                EncoderLabel { primary: c"Speed".to_owned(), secondary: c"Reboot".to_owned() },
                EncoderLabel { primary: c"Accel.".to_owned(), secondary: c"Zero".to_owned() },
            ],
            left_main: DisplayContent::TextLines {
                top_margin: 0,
                lines: vec![
                    c"Stroke".to_owned(),
                    c"{0} to {1}".to_owned(), // Start, End position
                    c"Speed  {2}".to_owned(), // Velocity
                    c"Accel. {3}".to_owned(), // Acceleration
                ],
            },
            right_main: DisplayContent::TextLines {
                top_margin: 11,
                lines: vec![
                    c"{4}".to_owned(),       // Current state OR error
                    c"{5} / {6}".to_owned(), // Actual, Demand position
                    c"{7}".to_owned(),       // Current OR warnings
                ],
            },
        };

        UiManager {
            io,
            active_screen: default_screen,
            motion_commands: DEFAULT_COMMANDS,
            last_sent_variables: HashMap::new(),
            holding_start: false,
        }
    }

    // TODO: It's unclear if this should be able to send ClientMessages back
    //       Without that, we've got some hacky logic in process_input_report instead
    pub fn process_core_message(&mut self, message: CoreMessage) -> Result<()> {
        if !matches!(message, CoreMessage::State { .. } | CoreMessage::CommandSetChanged { .. }) {
            trace!("process_core_message({:?}) called", message);
        }

        match message {
            CoreMessage::Connected { .. } => {
                // We're meant to send the variables first, but the firmware is tolerant of this
                self.io.send_report(OutputReport::ScreenSpec(self.active_screen.clone()))?;
                self.last_sent_variables.clear();
            }
            _ => {}
        }

        Ok(())
    }

    pub fn process_input_report(
        &mut self,
        report: InputReport,
        system_limits: &SystemLimits,
        core_state: &CoreState,
    ) -> Result<Vec<ClientMessage>> {
        if report.encoder_deltas != [0, 0, 0, 0] || !report.events.is_empty() {
            trace!("process_input_report({:?}) called", report);
        }

        if core_state.write_access_holder != Some(ControllerId::Hid) {
            // We're guaranteed to be able to claim write access, so this isn't totally awful
            return Ok(vec![
                ClientMessage::RequestWriteAccess { seq: 0 },
                ClientMessage::UpsertCommandSet {
                    seq: 0,
                    set: None,
                    base_version: None,
                    commands: self.motion_commands.to_vec(),
                },
            ]);
        }

        let mut messages = Vec::new();
        let mut variables = Vec::new();

        for (i, &delta) in report.encoder_deltas.iter().enumerate() {
            if delta == 0 {
                continue;
            }

            // TODO: The delta acceleration handling doesn't feel good at the HID tick rate, these values came from the LVGL tick
            let delta = delta as i32;
            match i {
                0 => {
                    self.motion_commands[0].position.0 = self.motion_commands[0]
                        .position
                        .0
                        .saturating_add(delta * delta.abs() * 10_000)
                        .clamp(0, system_limits.position.0);

                    messages.push(ClientMessage::UpdateCommand {
                        seq: 0,
                        update: CommandUpdate {
                            index: 0,
                            fields: MotionCommandFields {
                                position: Some(self.motion_commands[0].position),
                                velocity: None,
                                acceleration: None,
                                deceleration: None,
                            },
                        },
                    });

                    if self.holding_start || self.motion_commands[0].position > self.motion_commands[1].position {
                        if !self.holding_start {
                            self.motion_commands[1].position = self.motion_commands[0].position;
                        } else {
                            self.motion_commands[1].position.0 = self.motion_commands[1]
                                .position
                                .0
                                .saturating_add(delta * delta.abs() * 10_000)
                                .clamp(0, system_limits.position.0);
                        }

                        messages.push(ClientMessage::UpdateCommand {
                            seq: 0,
                            update: CommandUpdate {
                                index: 1,
                                fields: MotionCommandFields {
                                    position: Some(self.motion_commands[1].position),
                                    velocity: None,
                                    acceleration: None,
                                    deceleration: None,
                                },
                            },
                        });
                    }
                }
                1 => {
                    self.motion_commands[1].position.0 = self.motion_commands[1]
                        .position
                        .0
                        .saturating_add(delta * delta.abs() * 10_000)
                        .clamp(0, system_limits.position.0);

                    messages.push(ClientMessage::UpdateCommand {
                        seq: 0,
                        update: CommandUpdate {
                            index: 1,
                            fields: MotionCommandFields {
                                position: Some(self.motion_commands[1].position),
                                velocity: None,
                                acceleration: None,
                                deceleration: None,
                            },
                        },
                    });

                    if self.holding_start || self.motion_commands[1].position < self.motion_commands[0].position {
                        if !self.holding_start {
                            self.motion_commands[0].position = self.motion_commands[1].position;
                        } else {
                            self.motion_commands[0].position.0 = self.motion_commands[0]
                                .position
                                .0
                                .saturating_add(delta * delta.abs() * 10_000)
                                .clamp(0, system_limits.position.0);
                        }

                        messages.push(ClientMessage::UpdateCommand {
                            seq: 0,
                            update: CommandUpdate {
                                index: 0,
                                fields: MotionCommandFields {
                                    position: Some(self.motion_commands[0].position),
                                    velocity: None,
                                    acceleration: None,
                                    deceleration: None,
                                },
                            },
                        });
                    }
                }
                2 => {
                    self.motion_commands[0].velocity.0 = self.motion_commands[0]
                        .velocity
                        .0
                        .saturating_add(delta * delta.abs() * 10_000)
                        .clamp(0, system_limits.velocity.0);
                    self.motion_commands[1].velocity = self.motion_commands[0].velocity;

                    messages.push(ClientMessage::UpdateCommand {
                        seq: 0,
                        update: CommandUpdate {
                            index: 0,
                            fields: MotionCommandFields {
                                position: None,
                                velocity: Some(self.motion_commands[0].velocity),
                                acceleration: None,
                                deceleration: None,
                            },
                        },
                    });

                    messages.push(ClientMessage::UpdateCommand {
                        seq: 0,
                        update: CommandUpdate {
                            index: 1,
                            fields: MotionCommandFields {
                                position: None,
                                velocity: Some(self.motion_commands[1].velocity),
                                acceleration: None,
                                deceleration: None,
                            },
                        },
                    });
                }
                3 => {
                    self.motion_commands[0].acceleration.0 = self.motion_commands[0]
                        .acceleration
                        .0
                        .saturating_add(delta * delta.abs() * delta.abs() * 1_000)
                        .clamp(0, system_limits.acceleration.0);
                    self.motion_commands[1].acceleration = self.motion_commands[0].acceleration;
                    self.motion_commands[0].deceleration.0 =
                        self.motion_commands[0].acceleration.0.clamp(0, system_limits.deceleration.0);
                    self.motion_commands[1].deceleration = self.motion_commands[0].deceleration;

                    messages.push(ClientMessage::UpdateCommand {
                        seq: 0,
                        update: CommandUpdate {
                            index: 0,
                            fields: MotionCommandFields {
                                position: None,
                                velocity: None,
                                acceleration: Some(self.motion_commands[0].acceleration),
                                deceleration: Some(self.motion_commands[0].deceleration),
                            },
                        },
                    });

                    messages.push(ClientMessage::UpdateCommand {
                        seq: 0,
                        update: CommandUpdate {
                            index: 1,
                            fields: MotionCommandFields {
                                position: None,
                                velocity: None,
                                acceleration: Some(self.motion_commands[1].acceleration),
                                deceleration: Some(self.motion_commands[1].deceleration),
                            },
                        },
                    });
                }
                _ => {}
            }
        }

        for event in report.events {
            match event {
                HidInputEvent::ButtonClick { encoder_id } => {
                    if encoder_id == 0 {
                        match core_state.drive_state {
                            DriveState::Off => messages.push(ClientMessage::SetDrivePower { seq: 0, enabled: true }),
                            DriveState::Preparing | DriveState::Paused | DriveState::Moving => {
                                messages.push(ClientMessage::SetDrivePower { seq: 0, enabled: false })
                            }
                            DriveState::Errored => messages.push(ClientMessage::AcknowledgeError { seq: 0 }),
                            _ => {}
                        };
                    } else if encoder_id == 1 {
                        match core_state.drive_state {
                            DriveState::Paused => {
                                messages.push(ClientMessage::SetMotionState { seq: 0, action: MotionAction::Resume })
                            }
                            DriveState::Moving => {
                                messages.push(ClientMessage::SetMotionState { seq: 0, action: MotionAction::Pause })
                            }
                            _ => {}
                        };
                    } else if encoder_id == 2 {
                        variables.push(VariableEntry::HardwareControl(HardwareControl::Reset));
                    } else if encoder_id == 3 {
                        self.motion_commands = DEFAULT_COMMANDS;

                        messages.push(ClientMessage::UpsertCommandSet {
                            seq: 0,
                            set: None,
                            base_version: None,
                            commands: self.motion_commands.to_vec(),
                        });
                    }
                }
                HidInputEvent::ButtonHoldStart { encoder_id } => {
                    if encoder_id == 0 {
                        self.holding_start = true;
                    }
                }
                HidInputEvent::ButtonHoldEnd { encoder_id } => {
                    if encoder_id == 0 {
                        self.holding_start = false;
                    }
                }
                _ => {}
            }
        }

        // Be lazy, always calculate all the variables for now
        // We deduplicate them for sending, as otherwise it is noticeably slow
        {
            // LED rings
            let scale_to_u8 = |value: i32, limit: i32| -> u8 {
                if limit <= 0 {
                    return 0;
                }

                let scaled = (value.max(0).min(limit) as f64 / limit as f64) * 255.0;
                scaled.round().clamp(0.0, 255.0) as u8
            };

            variables.push(VariableEntry::HardwareControl(HardwareControl::LedRingValue {
                ring_id: 0,
                value: scale_to_u8(self.motion_commands[0].position.0, system_limits.position.0),
            }));
            variables.push(VariableEntry::HardwareControl(HardwareControl::LedRingValue {
                ring_id: 1,
                value: scale_to_u8(self.motion_commands[1].position.0, system_limits.position.0),
            }));
            variables.push(VariableEntry::HardwareControl(HardwareControl::LedRingValue {
                ring_id: 2,
                value: scale_to_u8(self.motion_commands[0].velocity.0, system_limits.velocity.0),
            }));
            variables.push(VariableEntry::HardwareControl(HardwareControl::LedRingValue {
                ring_id: 3,
                value: scale_to_u8(self.motion_commands[0].acceleration.0, system_limits.acceleration.0),
            }));

            // Screen variables
            // 0 = Start Position
            // 1 = End Position
            // 2 = Velocity
            // 3 = Acceleration
            // 4 = Error OR Current State
            // 5 = Actual Position
            // 6 = Demand Position
            // 7 = Warnings OR Motor Current
            // 8 = Enable / Disable / Acknowledge
            // 9 = Resume / Pause
            // 10 = Start / ""
            // 11 = End / Stroke
            variables.push(VariableEntry::FixedPoint {
                index: 0,
                decimals: 0,
                value: (self.motion_commands[0].position.0 / 10_000) as i16,
            });
            variables.push(VariableEntry::FixedPoint {
                index: 1,
                decimals: 0,
                value: (self.motion_commands[1].position.0 / 10_000) as i16,
            });
            variables.push(VariableEntry::FixedPoint {
                index: 2,
                decimals: 2,
                value: (self.motion_commands[0].velocity.0 / 10_000) as i16,
            });
            variables.push(VariableEntry::FixedPoint {
                index: 3,
                decimals: 2,
                value: (self.motion_commands[0].acceleration.0 / 1_000) as i16,
            });
            variables.push(VariableEntry::ShortString {
                index: 4,
                value: if let Some(error_code) = &core_state.error_code {
                    CString::new(format!("Error: {}", error_code)).unwrap()
                } else {
                    CString::new(format!("{:?}", core_state.drive_state)).unwrap()
                },
            });
            variables.push(VariableEntry::FixedPoint {
                index: 5,
                decimals: 2,
                value: (core_state.actual_position.0 / 100) as i16,
            });
            variables.push(VariableEntry::FixedPoint {
                index: 6,
                decimals: 2,
                value: (core_state.demand_position.0 / 100) as i16,
            });
            if !core_state.warnings.is_empty() {
                variables.push(VariableEntry::ShortString {
                    index: 7,
                    value: CString::new(core_state.warnings.join(", ")).unwrap(),
                });
            } else {
                variables.push(VariableEntry::FixedPoint {
                    index: 7,
                    decimals: 2,
                    value: core_state.current_draw.0 / 10,
                });
            }
            variables.push(VariableEntry::ShortString {
                index: 8,
                value: match &core_state.drive_state {
                    DriveState::Off => c"Enable".to_owned(),
                    DriveState::Preparing | DriveState::Paused | DriveState::Moving => c"Disable".to_owned(),
                    DriveState::Errored => c"Ack.".to_owned(),
                    _ => c"".to_owned(),
                },
            });
            variables.push(VariableEntry::ShortString {
                index: 9,
                value: match &core_state.drive_state {
                    DriveState::Paused => c"Resume".to_owned(),
                    DriveState::Moving => c"Pause".to_owned(),
                    _ => c"".to_owned(),
                },
            });
            variables.push(VariableEntry::ShortString {
                index: 10,
                value: if !self.holding_start { c"Start".to_owned() } else { c"".to_owned() },
            });
            variables.push(VariableEntry::ShortString {
                index: 11,
                value: if !self.holding_start { c"End".to_owned() } else { c"Stroke".to_owned() },
            });

            // Filter to only send the ones that have changed since the last time we sent a message
            variables.retain(|v| {
                let key = get_variable_key(v);
                match self.last_sent_variables.get(&key) {
                    Some(prev) if prev == v => false,
                    _ => {
                        self.last_sent_variables.insert(key, v.clone());
                        true
                    }
                }
            });
        }

        if !variables.is_empty() {
            self.io.send_report(OutputReport::VariableUpdate(variables))?;
        }

        Ok(messages)
    }
}

// TODO: Complete hack, come up with something better, probably a redesign of the types involved
fn get_variable_key(variable: &VariableEntry) -> u8 {
    match variable {
        VariableEntry::FixedPoint { index, .. } => *index,
        VariableEntry::ShortString { index, .. } => *index,
        VariableEntry::HardwareControl(hc) => {
            // HardwareControl entries have their own variable index space, so set the high bit
            0x80 | match hc {
                HardwareControl::LedRingValue { ring_id, .. } => *ring_id,
                HardwareControl::Reset => 5,
                HardwareControl::SleepLevel { .. } => 6,
            }
        }
    }
}
