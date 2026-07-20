pub mod buttons;
mod slint;
mod state;

use ::slint::platform::WindowAdapter;
use ::slint::{ComponentHandle, WindowSize};
pub use buttons::{Button, ButtonEvent, ButtonMap};
use std::sync::{mpsc, OnceLock};
use std::time::Duration;
use std::{cell::RefCell, rc::Rc, sync::mpsc::Receiver, time::Instant};

use ::slint::{
    platform::software_renderer::{LineBufferProvider, RepaintBufferType, TargetPixel},
    PhysicalSize, Timer,
};

use crate::device::drivers::lcd::RenderSource;
use crate::device::{Device, DisplayMode};

use self::{slint::Argb1555, slint::MinimalSoftwareWindow, state::UiState};

const DISPLAY_WIDTH: usize = 240;
const DISPLAY_HEIGHT: usize = 160;

#[derive(Debug)]
pub enum Message {
    /// The state of the buttons have changed.
    Button(ButtonMap),
    /// Battery status has changed.
    BatteryStatus { level: f32 },
    /// Redraw the entire screen (e.g. after a display change)
    Redraw,
    /// Go to the "Game" screen
    EnterGame,
    /// Game save persisted
    GameSaved,
    /// ROM loading progress
    RomLoadingProgress(f32),
    /// ROM select file list
    RomSelectFiles(Vec<(String, bool)>),
    /// ROM select error
    RomSelectError(String),
    /// Enter the error screen, and show the given error
    FatalError(String),
}

/// Send a message to the UI thread.
pub fn send(message: Message) {
    match SENDER.get() {
        Some(sender) => sender.send(message).unwrap(),
        None => log::error!("Dropping UI message {:?}", message),
    }
}

static SENDER: OnceLock<mpsc::Sender<Message>> = OnceLock::new();

/// Line renderer that renders to the FPGA
struct FpgaLineRenderer<'a, 'b, 'c> {
    device: &'a mut Device<'b>,
    line_buffer: &'c mut [Argb1555],
}

impl<'a, 'b, 'c> LineBufferProvider for &mut FpgaLineRenderer<'a, 'b, 'c> {
    type TargetPixel = Argb1555;

    fn process_line(
        &mut self,
        line: usize,
        range: core::ops::Range<usize>,
        render_fn: impl FnOnce(&mut [Self::TargetPixel]),
    ) {
        let start = ((DISPLAY_WIDTH * line) + range.start) * 2;
        let buffer = &mut self.line_buffer[range];
        render_fn(buffer);

        let slice = {
            let len = buffer.len() * 2;
            unsafe { std::slice::from_raw_parts(buffer.as_ptr() as *const u8, len) }
        };
        let _ = self.device.fpga.write_overlay(start as u32, slice);
    }
}

/// Line renderer that renders directly to the LCD
struct DirectLineRenderer<'a, 'b, 'c> {
    device: &'a mut Device<'b>,
    line_buffer: &'c mut [Argb1555],
    lcd_buffer: &'c mut [u8],
}

impl<'a, 'b, 'c> LineBufferProvider for &mut DirectLineRenderer<'a, 'b, 'c> {
    type TargetPixel = Argb1555;

    fn process_line(
        &mut self,
        line: usize,
        range: core::ops::Range<usize>,
        render_fn: impl FnOnce(&mut [Self::TargetPixel]),
    ) {
        let buffer = &mut self.line_buffer[range.clone()];
        render_fn(buffer);

        // Duplicate everything 2x in both directions
        // and convert from ARGB1555 to RGB888
        let mut lcd_index = 0;
        for x in buffer {
            for _ in 0..2 {
                self.lcd_buffer[lcd_index + 0] = x.red();
                self.lcd_buffer[lcd_index + 1] = x.green();
                self.lcd_buffer[lcd_index + 2] = x.blue();
                lcd_index += 3;
            }
        }
        let lcd_data = &self.lcd_buffer[0..((range.end - range.start) * 3 * 2)];
        for i in 0..2 {
            let _ = self.device.lcd.set_gram_pos(
                (range.start * 2) as u16,
                (range.end * 2) as u16,
                ((line * 2) + i) as u16,
                ((line * 2) + i + 1) as u16,
            );
            let _ = self.device.lcd.write_gram(lcd_data);
        }
    }
}

#[allow(unused)]
pub struct UI {
    framebuffer: Vec<Argb1555>,
    lcd_line_buffer: Vec<u8>,
    window: Rc<MinimalSoftwareWindow>,
    message_queue: Receiver<Message>,
    root: slint::MainWindow,
    state: Rc<RefCell<UiState>>,
    button_event_detector: buttons::ButtonEventDetector,
    boot_animation_end: Instant,
}

impl UI {
    pub fn new(device: &mut Device) -> Self {
        // Start the boot animation.
        device.fpga.write_u32(0xC000_0004, 36).unwrap(); // Set logo y
        let display_mode = if device.read_hdmi_detect().unwrap() {
            DisplayMode::External
        } else {
            DisplayMode::Internal
        };
        device.change_display_mode(display_mode).unwrap();
        device.fpga.write_u32(0xC000_0000, 1).unwrap(); // Start animation (no loop)
        let boot_animation_end = Instant::now() + Duration::from_secs(1);

        let (sender, receiver) = mpsc::channel::<Message>();
        SENDER.set(sender).expect("UI already initialized");

        let framebuffer = vec![Argb1555::from_rgb(0, 0, 0); DISPLAY_WIDTH];
        let lcd_line_buffer = vec![0u8; DISPLAY_WIDTH * 3 * 2];

        let window = MinimalSoftwareWindow::new(RepaintBufferType::ReusedBuffer);
        ::slint::platform::set_platform(Box::new(slint::HandheldPlatform {
            window: window.clone(),
        }))
        .unwrap();
        window.set_size(WindowSize::Physical(PhysicalSize::new(
            DISPLAY_WIDTH as u32,
            DISPLAY_HEIGHT as u32,
        )));

        let root = slint::MainWindow::new().unwrap();

        let ui = UI {
            framebuffer,
            lcd_line_buffer,
            window,
            message_queue: receiver,
            state: UiState::new(&root, device),
            root,
            button_event_detector: buttons::ButtonEventDetector::new(),
            boot_animation_end,
        };
        ui
    }

    pub fn run(&mut self) -> ! {
        // Set this thread (UI) to higher priority than background threads.
        unsafe { esp_idf_svc::sys::vTaskPrioritySet(std::ptr::null_mut(), 10) };

        let mut pending_message = None;
        loop {
            // Process messages.
            while let Some(message) = pending_message {
                self.dispatch_message(message);
                pending_message = self.message_queue.try_recv().ok();
            }
            for button_event in self.button_event_detector.update(None) {
                self.window.dispatch_event(button_event.into());
            }

            ::slint::platform::update_timers_and_animations();

            // Render UI if needed.
            self.window.draw_if_needed(|renderer| {
                let mut device = Device::lock();

                let render_start = Instant::now();
                let render_source = device.lcd.get_render_source();
                let device = match render_source {
                    RenderSource::Fpga => {
                        let mut line_renderer = FpgaLineRenderer {
                            device: &mut device,
                            line_buffer: &mut self.framebuffer,
                        };
                        renderer.render_by_line(&mut line_renderer);

                        line_renderer.device
                    }
                    RenderSource::Mcu => {
                        let mut line_renderer = DirectLineRenderer {
                            device: &mut device,
                            line_buffer: &mut self.framebuffer,
                            lcd_buffer: &mut self.lcd_line_buffer,
                        };
                        renderer.render_by_line(&mut line_renderer);
                        line_renderer.device
                    }
                };
                let render_duration = render_start.elapsed();
                log::info!("Render + display {}ms", render_duration.as_millis() as u32,);

                // XXX: only need to do this when switching overlays
                let _ = device
                    .fpga
                    .set_overlay_bounds(0x0, 0xFF, 0x0, 0x0, 0xFF, 0x0);

                // If we changed the repaint buffer type to force a redraw, change it back.
                if renderer.repaint_buffer_type() == RepaintBufferType::NewBuffer {
                    renderer.set_repaint_buffer_type(RepaintBufferType::ReusedBuffer);
                    // For some reason, this doesn't get called automatically after the first render.
                    self.window.request_redraw();
                }
            });

            // Trigger a timer to wake us up for button repeat events.
            if let Some(wakeup) = self.button_event_detector.next_wakeup_time() {
                Timer::single_shot(wakeup.saturating_duration_since(Instant::now()), || ());
            }

            // Sleep until the next animation, timer, or event.
            if !self.window.has_active_animations() {
                match ::slint::platform::duration_until_next_timer_update() {
                    Some(duration) => {
                        pending_message = self.message_queue.recv_timeout(duration).ok();
                    }
                    None => {
                        pending_message = self.message_queue.recv().ok();
                    }
                }
            }
        }
    }

    /// Handle a message sent to the UI thread.
    fn dispatch_message(&mut self, message: Message) {
        match message {
            Message::Button(state) => {
                for button_event in self.button_event_detector.update(Some(state)) {
                    self.window.dispatch_event(button_event.into());
                }
            }
            Message::BatteryStatus { level } => {
                self.state.borrow_mut().update_battery_level(level);
            }
            Message::Redraw => {
                log::info!("Refreshing screen");
                self.window.request_redraw();

                // Force renderer to clear the dirty region and re-render everything.
                self.window
                    .renderer
                    .set_repaint_buffer_type(RepaintBufferType::NewBuffer);
            }
            Message::EnterGame => {
                self.root.invoke_set_screen(slint::ScreenId::Game);
            }
            Message::GameSaved => {
                self.state.borrow_mut().game_on_saved();
            }
            Message::RomLoadingProgress(progress) => {
                self.root
                    .global::<slint::Backend>()
                    .set_rom_select_progress(progress * 100.0);
            }
            Message::RomSelectFiles(files) => {
                self.state.borrow_mut().rom_select_update_list(files);
            }
            Message::RomSelectError(error) => {
                self.state.borrow_mut().rom_select_set_error(error);
            }
            Message::FatalError(error) => {
                self.root
                    .global::<slint::Backend>()
                    .set_error_text(error.into());
                self.root.invoke_set_screen(slint::ScreenId::Error);
            }
            #[allow(unreachable_patterns)]
            _ => {
                log::warn!("Unhandled message: {:?}", message);
            }
        }
    }
}
