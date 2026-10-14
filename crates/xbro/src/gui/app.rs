//! winit glue: window and surface, event translation, redraw scheduling.

use std::num::NonZeroU32;
use std::rc::Rc;
use std::time::Instant;

use softbuffer::{Context, Surface};
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::{ElementState, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoopProxy};
use winit::keyboard::ModifiersState;
use winit::window::{CursorIcon, Window, WindowId};
use xbro_engine::{Cursor, Engine, Url};

use super::browser::{Browser, Metrics};
use super::clipboard::SystemClipboard;
use super::events::{MouseButton, WheelDelta};
use super::input::{KeyInput, Mods};
use super::paint::rgba_to_xrgb;
use super::render::Renderer;
use super::{loader, UserEvent};
use crate::Options;

const MIN_WIDTH: f64 = 400.0;
const MIN_HEIGHT: f64 = 300.0;
const DEFAULT_HEIGHT: u32 = 768;
const MAX_INITIAL_SIZE: u32 = 8_192;

struct WindowState {
    window: Rc<Window>,
    surface: Surface<Rc<Window>, Rc<Window>>,
    /// Size the surface buffer was last given.
    surface_size: Option<(NonZeroU32, NonZeroU32)>,
}

pub struct App {
    engine: Engine,
    proxy: EventLoopProxy<UserEvent>,
    initial_size: (u32, u32),
    initial_url: Option<Url>,
    browser: Option<Browser>,
    window: Option<WindowState>,
    renderer: Renderer,
    modifiers: ModifiersState,
    clipboard: SystemClipboard,
    error: Option<String>,
    title: String,
    cursor: Option<Cursor>,
    /// Set once the window is being closed: it is not touched any more.
    closing: bool,
    /// Whether a drawing error was already reported (they tend to repeat).
    draw_error_reported: bool,
}

impl App {
    pub fn new(opts: &Options, url: Url, proxy: EventLoopProxy<UserEvent>) -> Self {
        let height = if opts.height == 0 {
            DEFAULT_HEIGHT
        } else {
            opts.height
        };
        Self {
            engine: Engine::new(),
            proxy,
            initial_size: (
                opts.width.clamp(1, MAX_INITIAL_SIZE),
                height.clamp(1, MAX_INITIAL_SIZE),
            ),
            initial_url: Some(url),
            browser: None,
            window: None,
            renderer: Renderer::default(),
            modifiers: ModifiersState::empty(),
            clipboard: SystemClipboard::default(),
            error: None,
            title: String::new(),
            cursor: None,
            closing: false,
            draw_error_reported: false,
        }
    }

    /// Why the window could not be opened, if it could not.
    pub fn take_error(&mut self) -> Option<String> {
        self.error.take()
    }

    fn create_window(&self, event_loop: &ActiveEventLoop) -> Result<WindowState, String> {
        let (w, h) = self.initial_size;
        let attributes = Window::default_attributes()
            .with_title("xbro")
            .with_inner_size(LogicalSize::new(f64::from(w), f64::from(h)))
            .with_min_inner_size(LogicalSize::new(MIN_WIDTH, MIN_HEIGHT));
        let window = Rc::new(
            event_loop
                .create_window(attributes)
                .map_err(|e| e.to_string())?,
        );
        let context = Context::new(window.clone()).map_err(|e| e.to_string())?;
        let surface = Surface::new(&context, window.clone()).map_err(|e| e.to_string())?;
        Ok(WindowState {
            window,
            surface,
            surface_size: None,
        })
    }

    fn mods(&self) -> Mods {
        Mods::from_state(self.modifiers)
    }

    /// Applies the browser's state to the window: exit, title, cursor, redraw.
    fn sync(&mut self, event_loop: &ActiveEventLoop) {
        let (Some(browser), Some(state)) = (self.browser.as_mut(), self.window.as_ref()) else {
            return;
        };
        if self.closing || browser.quit {
            self.closing = true;
            event_loop.exit();
            return;
        }
        let title = browser.window_title();
        if title != self.title {
            state.window.set_title(&title);
            self.title = title;
        }
        if self.cursor != Some(browser.cursor) {
            state.window.set_cursor(cursor_icon(browser.cursor));
            self.cursor = Some(browser.cursor);
        }
        if browser.take_needs_redraw() {
            state.window.request_redraw();
        }
    }

    /// Draws and presents a frame at the size last reported by the window
    /// system (the window is not queried: on X11 that panics in winit when
    /// the window was destroyed behind our back).
    fn redraw(&mut self) {
        if self.closing {
            return;
        }
        let (Some(browser), Some(state)) = (self.browser.as_mut(), self.window.as_mut()) else {
            return;
        };
        let (Some(width), Some(height)) = (
            NonZeroU32::new(browser.metrics.width),
            NonZeroU32::new(browser.metrics.height),
        ) else {
            return;
        };
        let result = (|| -> Result<(), String> {
            if state.surface_size != Some((width, height)) {
                state
                    .surface
                    .resize(width, height)
                    .map_err(|e| format!("cannot resize the window surface: {e}"))?;
                state.surface_size = Some((width, height));
            }
            let Some(frame) = self.renderer.render(browser, Instant::now()) else {
                return Ok(());
            };
            let mut buffer = state
                .surface
                .buffer_mut()
                .map_err(|e| format!("cannot draw the window: {e}"))?;
            rgba_to_xrgb(frame.data(), &mut buffer);
            buffer
                .present()
                .map_err(|e| format!("cannot present the frame: {e}"))
        })();
        if let Err(e) = result {
            if !std::mem::replace(&mut self.draw_error_reported, true) {
                eprintln!("xbro: {e}");
            }
        }
    }
}

impl ApplicationHandler<UserEvent> for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let state = match self.create_window(event_loop) {
            Ok(state) => state,
            Err(e) => {
                self.error = Some(e);
                event_loop.exit();
                return;
            }
        };
        let size = state.window.inner_size();
        let metrics = Metrics {
            width: size.width,
            height: size.height,
            dpi: state.window.scale_factor() as f32,
        };
        match self.browser.as_mut() {
            Some(browser) => browser.resize(metrics.width, metrics.height, metrics.dpi),
            None => {
                let (engine, proxy) = (self.engine.clone(), self.proxy.clone());
                let loader = Box::new(move |job| loader::spawn(engine.clone(), job, proxy.clone()));
                let mut browser = Browser::new(self.engine.fonts.clone(), metrics, loader);
                if let Some(url) = self.initial_url.take() {
                    browser.open_initial(url);
                }
                self.browser = Some(browser);
            }
        }
        state.window.request_redraw();
        self.window = Some(state);
        self.sync(event_loop);
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: UserEvent) {
        let Some(browser) = self.browser.as_mut() else {
            return;
        };
        match event {
            UserEvent::Loaded {
                tab_id,
                nav_id,
                page,
            } => browser.on_loaded(tab_id, nav_id, *page),
        }
        self.sync(event_loop);
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        if self.closing {
            return;
        }
        let mods = self.mods();
        let (Some(browser), Some(state)) = (self.browser.as_mut(), self.window.as_ref()) else {
            return;
        };
        match event {
            WindowEvent::CloseRequested | WindowEvent::Destroyed => {
                self.closing = true;
                event_loop.exit();
                return;
            }
            // Minimised windows report 0×0: keep the last layout.
            WindowEvent::Resized(size) if size.width > 0 && size.height > 0 => {
                browser.resize(size.width, size.height, state.window.scale_factor() as f32)
            }
            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                // The new size follows in a `Resized` event.
                let (width, height) = (browser.metrics.width, browser.metrics.height);
                browser.resize(width, height, scale_factor as f32);
            }
            WindowEvent::RedrawRequested => {
                self.redraw();
                return;
            }
            WindowEvent::ModifiersChanged(m) => self.modifiers = m.state(),
            WindowEvent::Focused(false) => self.modifiers = ModifiersState::empty(),
            WindowEvent::KeyboardInput {
                event,
                is_synthetic,
                ..
            } => {
                if event.state == ElementState::Pressed && !is_synthetic {
                    let key = KeyInput {
                        key: &event.logical_key,
                        code: event.physical_key,
                        text: event.text.as_deref(),
                        mods,
                    };
                    browser.key(&key, &mut self.clipboard);
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                browser.mouse_move(position.x as f32, position.y as f32)
            }
            WindowEvent::CursorLeft { .. } => browser.mouse_left(),
            WindowEvent::MouseInput { state, button, .. } => {
                let button = match button {
                    winit::event::MouseButton::Left => MouseButton::Left,
                    winit::event::MouseButton::Middle => MouseButton::Middle,
                    winit::event::MouseButton::Right => MouseButton::Right,
                    winit::event::MouseButton::Back => MouseButton::Back,
                    winit::event::MouseButton::Forward => MouseButton::Forward,
                    winit::event::MouseButton::Other(_) => return,
                };
                browser.mouse_button(button, state == ElementState::Pressed, mods);
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let delta = match delta {
                    MouseScrollDelta::LineDelta(x, y) => WheelDelta::Lines(x, y),
                    MouseScrollDelta::PixelDelta(p) => WheelDelta::Pixels(p.x as f32, p.y as f32),
                };
                browser.wheel(delta, mods);
            }
            _ => {}
        }
        self.sync(event_loop);
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        // Timers are checked after every batch of events, not only when the
        // wait times out: a stream of input events must not starve them.
        let now = Instant::now();
        if let Some(browser) = self.browser.as_mut() {
            browser.tick(now);
        }
        self.sync(event_loop);
        let flow = match self.browser.as_ref().and_then(|b| b.next_deadline(now)) {
            Some(deadline) => ControlFlow::WaitUntil(deadline),
            None => ControlFlow::Wait,
        };
        event_loop.set_control_flow(flow);
    }
}

fn cursor_icon(cursor: Cursor) -> CursorIcon {
    match cursor {
        Cursor::Auto | Cursor::Default => CursorIcon::Default,
        Cursor::Pointer => CursorIcon::Pointer,
        Cursor::Text => CursorIcon::Text,
        Cursor::Wait => CursorIcon::Wait,
        Cursor::Progress => CursorIcon::Progress,
        Cursor::Help => CursorIcon::Help,
        Cursor::Move => CursorIcon::Move,
        Cursor::NotAllowed => CursorIcon::NotAllowed,
        Cursor::Crosshair => CursorIcon::Crosshair,
        Cursor::Grab => CursorIcon::Grab,
    }
}
