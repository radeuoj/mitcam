use std::net::SocketAddr;
use tokio::net::{TcpListener, TcpStream};
use nokhwa::pixel_format::RgbAFormat;
use nokhwa::utils::{ApiBackend, CameraIndex, RequestedFormat, RequestedFormatType};
use nokhwa::Camera;
use std::num::NonZeroU32;
use std::rc::Rc;
use std::sync::RwLock;
use futures_util::{StreamExt, TryStreamExt};
use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, EventLoop, OwnedDisplayHandle};
use winit::window::{Window, WindowId};

struct App {
    window: Option<Rc<Window>>,
    surface: Option<softbuffer::Surface<OwnedDisplayHandle, Rc<Window>>>,
    camera: Camera,
    front_buffer: RwLock<Vec<u8>>,
    back_buffer: Vec<u8>,
}

type Decoder = RgbAFormat;
impl App {
    const CAMERA_INDEX: u32 = 1;

    fn new() -> Self {
        let index = CameraIndex::Index(Self::CAMERA_INDEX);
        let format = RequestedFormat::new::<Decoder>(RequestedFormatType::AbsoluteHighestFrameRate);
        let camera = Camera::new(index, format).unwrap();

        let resolution = camera.resolution();
        let size = (resolution.width() * resolution.height()) as usize;

        let mut front_buffer = Vec::new();
        front_buffer.resize(size * 4, 0);
        let mut back_buffer = Vec::new();
        back_buffer.resize(size * 4, 0);

        Self {
            window: None,
            surface: None,
            camera,
            front_buffer: RwLock::new(front_buffer),
            back_buffer,
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
                {
                    self.camera.write_frame_to_buffer::<Decoder>(&mut self.back_buffer).unwrap();
                    let mut front_buffer = self.front_buffer.write().unwrap();
                    std::mem::swap(&mut self.back_buffer, &mut front_buffer);
                }

                let mut buffer = self.surface.as_mut().unwrap().buffer_mut().unwrap();
                let front_buffer = self.front_buffer.read().unwrap();
                for (screen, chunk) in buffer.iter_mut().zip(front_buffer.as_chunks::<4>().0.iter()) {
                    *screen = (chunk[0] as u32) << 16 | (chunk[1] as u32) << 8 | (chunk[2] as u32);
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

async fn handle_connection(stream: TcpStream, addr: SocketAddr) {
    let stream = tokio_tungstenite::accept_async(stream).await.unwrap();
    println!("WebSocket connection established: {}", addr);

    let (outgoing, incoming) = stream.split();

    incoming.try_for_each(|msg| {
        println!("Received a message from {}: {}", addr, msg.to_text().unwrap());
        futures_util::future::ok(())
    }).await.unwrap();
}

#[tokio::main]
async fn main() {
    print_available_cameras();

    tokio::spawn(async move {
        let socket = TcpListener::bind("0.0.0.0:7878").await.unwrap();
        while let Ok((stream, addr)) = socket.accept().await {
            tokio::spawn(handle_connection(stream, addr));
        }
    });

    let event_loop = EventLoop::new().unwrap();
    event_loop.run_app(&mut App::new()).unwrap();


}
