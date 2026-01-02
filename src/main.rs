use nokhwa::pixel_format::RgbAFormat;
use nokhwa::utils::{ApiBackend, CameraIndex, RequestedFormat, RequestedFormatType};
use nokhwa::Camera;
use std::num::NonZeroU32;
use std::rc::Rc;
use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, EventLoop, OwnedDisplayHandle};
use winit::window::{Window, WindowId};

struct App {
    window: Option<Rc<Window>>,
    surface: Option<softbuffer::Surface<OwnedDisplayHandle, Rc<Window>>>,
    camera: Camera,
}

type Decoder = RgbAFormat;
impl App {
    const CAMERA_INDEX: u32 = 1;

    fn new() -> Self {
        let index = CameraIndex::Index(Self::CAMERA_INDEX);
        let format = RequestedFormat::new::<Decoder>(RequestedFormatType::AbsoluteHighestFrameRate);
        let camera = Camera::new(index, format).unwrap();

        Self {
            window: None,
            surface: None,
            camera,
        }
    }

    fn get_camera_resolution(&self) -> (u32, u32) {
        let resolution = self.camera.resolution();
        (resolution.width(), resolution.height())
    }
}

impl Drop for App {
    fn drop(&mut self) {
        self.camera.stop_stream().unwrap();
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        let window_attributes = Window::default_attributes()
            .with_title("Mitcam")
            .with_inner_size::<winit::dpi::LogicalSize<u32>>(self.get_camera_resolution().into())
            .with_resizable(false);
        self.window = Some(Rc::new(event_loop.create_window(window_attributes).unwrap()));

        let context = softbuffer::Context::new(event_loop.owned_display_handle()).unwrap();
        self.surface = Some(softbuffer::Surface::new(&context, self.window.as_ref().unwrap().clone()).unwrap());
        let (width, height) = self.get_camera_resolution();
        self.surface.as_mut().unwrap().resize(NonZeroU32::new(width).unwrap(), NonZeroU32::new(height).unwrap()).unwrap();

        self.camera.open_stream().unwrap();
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _window_id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::RedrawRequested => {
                let mut buffer = self.surface.as_mut().unwrap().buffer_mut().unwrap();
                {
                    let buffer: &mut [u8] = unsafe {
                        let len = 4 * buffer.len();
                        let ptr = buffer.as_ptr() as *mut u8;
                        std::slice::from_raw_parts_mut(ptr, len)
                    };
                    self.camera.write_frame_to_buffer::<Decoder>(buffer).unwrap();
                }

                for val in buffer.iter_mut() {
                    let r = *val & 0xff;
                    *val >>= 8;

                    let g = *val & 0xff;
                    *val >>= 8;

                    let b = *val & 0xff;
                    *val >>= 8;

                    *val = r << 16 | g << 8 | b;
                }

                buffer.present().unwrap();

                self.window.as_mut().unwrap().request_redraw();
            }
            _ => {}
        }
    }
}

fn print_available_cameras() {
    println!("Available cameras:");
    for info in nokhwa::query(ApiBackend::Auto).unwrap() {
        println!("{} -> {}", info.index(), info.human_name());
    }
}

fn main() {
    print_available_cameras();
    let event_loop = EventLoop::new().unwrap();
    event_loop.run_app(&mut App::new()).unwrap();
}
