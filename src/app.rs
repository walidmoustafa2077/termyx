use std::num::NonZeroU32;
use std::sync::Arc;

use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, EventLoopProxy};
use winit::window::{Window, WindowId};

use crate::config::Config;
use crate::event::UserEvent;
use crate::render::Renderer;
use crate::theme::Theme;

pub struct App {
    config: Config,
    _proxy: EventLoopProxy<UserEvent>,
    window: Option<Arc<Window>>,
    surface: Option<softbuffer::Surface<Arc<Window>, Arc<Window>>>,
    renderer: Option<Renderer>,
}

impl App {
    pub fn new(config: Config, proxy: EventLoopProxy<UserEvent>) -> Self {
        Self {
            config,
            _proxy: proxy,
            window: None,
            surface: None,
            renderer: None,
        }
    }
}

impl ApplicationHandler<UserEvent> for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let attributes = Window::default_attributes()
            .with_title("Termyx")
            .with_inner_size(LogicalSize::new(
                self.config.window.width,
                self.config.window.height,
            ));
        let window = Arc::new(event_loop.create_window(attributes).expect("create window"));
        let context = softbuffer::Context::new(window.clone()).expect("softbuffer context");
        let surface =
            softbuffer::Surface::new(&context, window.clone()).expect("softbuffer surface");
        let mut renderer = Renderer::new(
            Theme::from_config(&self.config.theme),
            self.config.window.padding,
        );
        let size = window.inner_size();
        renderer.resize(size.width, size.height);
        self.renderer = Some(renderer);
        self.surface = Some(surface);
        self.window = Some(window);
        self.window.as_ref().unwrap().request_redraw();
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                if let (Some(surface), Some(renderer)) =
                    (self.surface.as_mut(), self.renderer.as_mut())
                {
                    if let (Some(width), Some(height)) =
                        (NonZeroU32::new(size.width), NonZeroU32::new(size.height))
                    {
                        let _ = surface.resize(width, height);
                        renderer.resize(size.width, size.height);
                    }
                }
                if let Some(window) = &self.window {
                    window.request_redraw();
                }
            }
            WindowEvent::RedrawRequested => {
                let (Some(surface), Some(renderer)) =
                    (self.surface.as_mut(), self.renderer.as_mut())
                else {
                    return;
                };
                renderer.draw();
                let Ok(mut buffer) = surface.buffer_mut() else {
                    return;
                };
                buffer.copy_from_slice(renderer.pixels());
                let _ = buffer.present();
            }
            _ => {}
        }
    }
}
