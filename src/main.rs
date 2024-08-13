use anyhow::{Result, Context};
use crossbeam_channel::{unbounded, Receiver};
use iced::{executor, Application, Command, Element, Settings, Slider, Column, Text};
use std::{thread, sync::Arc};
use std::sync::Mutex;
use systray::Application as SystrayApplication;
use winapi::um::winuser::{EnumDisplayMonitors, CreateWindowExW, SetLayeredWindowAttributes, ShowWindow, SW_SHOW, SW_HIDE};
use winapi::um::wingdi::RGB;
use winapi::um::physicalmonitorenumerationapi::{GetPhysicalMonitorsFromHMONITOR, PHYSICAL_MONITOR};
use winapi::um::highlevelmonitorconfigurationapi::SetMonitorBrightness;
use winapi::shared::windef::{HMONITOR, HDC, LPRECT, HWND};
use winapi::shared::minwindef::BOOL;
use std::ptr;

struct LuxFlex {
    brightness: u8,
    dimmer_alpha: u8,
    dimmer_hwnd: HWND,
    receiver: Receiver<SystrayMessage>,
    window_visible: bool,
}

#[derive(Debug, Clone)]
enum Message {
    SliderChanged(u8),
    ToggleVisibility,
}

#[derive(Debug, Clone)]
enum SystrayMessage {
    ShowControls,
    Quit,
}

impl iced::Application for LuxFlex {
    type Executor = executor::Default;
    type Message = Message;
    type Flags = Receiver<SystrayMessage>;

    fn new(flags: Self::Flags) -> (Self, Command<Message>) {
        let mut app = Self {
            brightness: 50.0,
            dimmer_alpha: 0,
            dimmer_hwnd: ptr::null_mut(),
            receiver: flags,
            window_visible: false,
        };
        app.create_dimmer_window().expect("Failed to create dimmer window");
        (app, Command::none())
    }

    fn title(&self) -> String {
        String::from("LuxFlex")
    }

    fn update(&mut self, message: Message) -> Command<Message> {
        match message {
            Message::SliderChanged(value) => {
                self.update_from_slider(value as i32).expect("Failed to update from slider");
            }
            Message::ToggleVisibility => {
                self.window_visible = !self.window_visible;
                if self.window_visible {
                    unsafe { ShowWindow(self.dimmer_hwnd, SW_SHOW); }
                } else {
                    unsafe { ShowWindow(self.dimmer_hwnd, SW_HIDE); }
                }
            }
        }
        Command::none()
    }

    fn view(&mut self) -> Element<Message> {
        Column::new()
            .push(Text::new("Brightness/Dimness"))
            .push(Slider::new(
                0..=100,
                self.brightness as u8,
                Message::SliderChanged,
            ))
            .into()
    }

    fn subscription(&self) -> iced::Subscription<Message> {
        iced::Subscription::unfold(
            "systray_events",
            self.receiver.clone(),
            |mut receiver| async move {
                if let Ok(msg) = receiver.recv() {
                    match msg {
                        SystrayMessage::ShowControls => Some((Message::ToggleVisibility, receiver)),
                        SystrayMessage::Quit => std::process::exit(0),
                    }
                } else {
                    None
                }
            },
        )
    }
}

impl LuxFlex {
    fn set_brightness(&mut self, brightness: u32) -> Result<()> {
        unsafe {
            EnumDisplayMonitors(ptr::null_mut(), ptr::null(), Some(enum_monitor), brightness as isize);
        }
        self.brightness = brightness as f32;
        Ok(())
    }

    fn set_dimmer(&mut self, alpha: u8) -> Result<()> {
        unsafe {
            SetLayeredWindowAttributes(self.dimmer_hwnd, RGB(0, 0, 0), alpha, 2);
        }
        self.dimmer_alpha = alpha;
        Ok(())
    }

    fn create_dimmer_window(&mut self) -> Result<()> {
        use winapi::um::winuser::{WS_EX_LAYERED, WS_EX_TRANSPARENT, WS_EX_TOPMOST, WS_POPUP};
        use winapi::um::winuser::{GetSystemMetrics, SM_CXSCREEN, SM_CYSCREEN};

        unsafe {
            self.dimmer_hwnd = CreateWindowExW(
                WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_TOPMOST,
                wide_string("Static").as_ptr(),
                ptr::null(),
                WS_POPUP,
                0, 0, GetSystemMetrics(SM_CXSCREEN), GetSystemMetrics(SM_CYSCREEN),
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null_mut()
            );

            if self.dimmer_hwnd.is_null() {
                anyhow::bail!("Failed to create dimmer window");
            }

            SetLayeredWindowAttributes(self.dimmer_hwnd, RGB(0, 0, 0), 0, 2);
            ShowWindow(self.dimmer_hwnd, SW_HIDE);
        }
        Ok(())
    }

    fn update_from_slider(&mut self, value: i32) -> Result<()> {
        if value <= 50 {
            self.set_brightness((value * 2) as u32)?;
            self.set_dimmer(0)?;
        } else {
            self.set_brightness(100)?;
            self.set_dimmer(((value - 50) * 5) as u8)?;
        }
        Ok(())
    }
}

unsafe extern "system" fn enum_monitor(hmonitor: HMONITOR, _: HDC, _: LPRECT, brightness: isize) -> BOOL {
    let mut physical_monitor = PHYSICAL_MONITOR::default();
    let monitor_count = 1;
    
    if GetPhysicalMonitorsFromHMONITOR(hmonitor, monitor_count, &mut physical_monitor) != 0 {
        SetMonitorBrightness(physical_monitor.hPhysicalMonitor, brightness as u32);
    }
    
    1 // Continue enumeration
}

fn wide_string(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn main() -> Result<()> {
    let (tx, rx) = unbounded();
    let tx = Arc::new(Mutex::new(tx));

    thread::spawn(move || {
        let mut systray = SystrayApplication::new().expect("Failed to create systray app");
        
        let tx_clone = Arc::clone(&tx);
        systray.add_menu_item("Show/Hide Controls", move |_| {
            tx_clone.lock().unwrap().send(SystrayMessage::ShowControls).unwrap();
        }).unwrap();

        let tx_clone = Arc::clone(&tx);
        systray.add_menu_item("Quit", move |_| {
            tx_clone.lock().unwrap().send(SystrayMessage::Quit).unwrap();
        }).unwrap();

        systray.wait_for_message().unwrap();
    });

    LuxFlex::run(Settings::with_flags(rx)).context("Failed to run iced app")?;

    Ok(())
}