use axum::body::Bytes;
use axum::extract::ws::{Message, WebSocket};
use axum::extract::{ConnectInfo, WebSocketUpgrade};
use axum::routing::any;
use axum::{Router, ServiceExt};
use axum_extra::TypedHeader;
use futures_util::{SinkExt, StreamExt};
use nokhwa::pixel_format::RgbAFormat;
use nokhwa::utils::{ApiBackend, CameraIndex, RequestedFormat, RequestedFormatType};
use nokhwa::Camera;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::num::NonZeroU32;
use std::process::exit;
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;
use tokio::net::TcpListener;
use tower_http::services::ServeDir;
use tower_http::trace::{DefaultMakeSpan, TraceLayer};
use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, EventLoop, EventLoopProxy, OwnedDisplayHandle};
use winit::window::{Window, WindowId};

const CAMERA_INDEX: u32 = 1;
const SLEEP_TIME: Duration = Duration::from_millis(100);
type Decoder = RgbAFormat;
type Tx = std::sync::mpsc::Sender<Bytes>;
type PeerMap = Arc<Mutex<HashMap<SocketAddr, Tx>>>;

struct App<'a> {
    window: Option<Arc<Window>>,
    surface: Option<softbuffer::Surface<OwnedDisplayHandle, Arc<Window>>>,
    front_buffer: &'a RwLock<Vec<u8>>,
    size: (u32, u32),
}

#[derive(Copy, Clone, Debug)]
enum UserEvent {
    Redraw,
}

impl<'a> App<'a> {
    fn new(size: (u32, u32), front_buffer: &'a RwLock<Vec<u8>>) -> App<'a> {
        Self {
            window: None,
            surface: None,
            front_buffer,
            size,
        }
    }
}

impl ApplicationHandler<UserEvent> for App<'_> {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        let window_attributes = Window::default_attributes()
            .with_title("Mitcam")
            .with_inner_size::<winit::dpi::PhysicalSize<u32>>(self.size.into())
            .with_resizable(false);
        self.window = Some(Arc::new(event_loop.create_window(window_attributes).unwrap()));

        let context = softbuffer::Context::new(event_loop.owned_display_handle()).unwrap();
        self.surface = Some(softbuffer::Surface::new(&context, self.window.as_ref().unwrap().clone()).unwrap());
        let (width, height) = self.size;
        self.surface.as_mut().unwrap().resize(NonZeroU32::new(width).unwrap(), NonZeroU32::new(height).unwrap()).unwrap();
    }

    fn user_event(&mut self, _event_loop: &ActiveEventLoop, event: UserEvent) {
        match event {
            UserEvent::Redraw => self.window.as_ref().unwrap().request_redraw(),
        }
    }

    fn window_event(&mut self, _event_loop: &ActiveEventLoop, _window_id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => exit(0),
            WindowEvent::RedrawRequested => {
                let mut buffer = self.surface.as_mut().unwrap().buffer_mut().unwrap();
                let front_buffer = self.front_buffer.read().unwrap();

                for (screen, chunk) in buffer.iter_mut().zip(front_buffer.as_chunks::<4>().0.iter()) {
                    *screen = (chunk[0] as u32) << 16 | (chunk[1] as u32) << 8 | (chunk[2] as u32);
                }

                buffer.present().unwrap();
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

fn make_camera() -> Camera {
    let index = CameraIndex::Index(CAMERA_INDEX);
    let format = RequestedFormat::new::<Decoder>(RequestedFormatType::AbsoluteHighestFrameRate);
    Camera::new(index, format).unwrap()
}

fn get_camera_resolution(camera: &Camera) -> (u32, u32) {
    let resolution = camera.resolution();
    (resolution.width(), resolution.height())
}

fn make_buffer(size: (u32, u32)) -> Vec<u8> {
    let mut buffer = Vec::new();
    buffer.resize((4 * size.0 * size.1) as usize, 0);
    buffer
}

fn run_camera(front_buffer: &RwLock<Vec<u8>>) {
    let mut camera = make_camera();
    camera.open_stream().unwrap();
    let mut back_buffer = make_buffer(get_camera_resolution(&camera));

    loop {
        camera.write_frame_to_buffer::<Decoder>(&mut back_buffer).unwrap();
        let mut front_buffer = front_buffer.write().unwrap();
        std::mem::swap(&mut back_buffer, &mut front_buffer);

        std::thread::sleep(SLEEP_TIME);
    }
}

async fn run_web_server(peer_map: PeerMap) {
    let router = Router::new()
        .route("/ws", any(async move |ws: WebSocketUpgrade, _user_agent: Option<TypedHeader<headers::UserAgent>>, ConnectInfo(addr): ConnectInfo<SocketAddr>| {
            ws.on_upgrade(move |socket| handle_connection(socket, addr, peer_map))
        }))
        .fallback_service(ServeDir::new("web"))
        .layer(TraceLayer::new_for_http().make_span_with(DefaultMakeSpan::default().include_headers(true)));

    let listener = TcpListener::bind("0.0.0.0:5555").await.unwrap();
    println!("Listening on: {}", listener.local_addr().unwrap());
    axum::serve(listener, router.into_make_service_with_connect_info::<SocketAddr>()).await.unwrap();
}

async fn handle_connection(socket: WebSocket, addr: SocketAddr, peer_map: PeerMap) {
    println!("{addr} connected to the websocket");

    let (mut sender, mut receiver) = socket.split();
    let (tx, rx) = std::sync::mpsc::channel::<Bytes>();
    peer_map.lock().unwrap().insert(addr, tx);

    let mut recv_task = tokio::task::spawn(async move {
        while let Some(Ok(msg)) = receiver.next().await {
            match msg {
                Message::Close(_) => break,
                _ => {}
            }
        }
    });

    let mut send_task = tokio::task::spawn(async move {
        while let Ok(bytes) = rx.recv() {
            if sender.send(Message::Binary(bytes)).await.is_err() {
                break;
            }
        }
    });

    tokio::select! {
        _ = &mut send_task => recv_task.abort(),
        _ = &mut recv_task => send_task.abort(),
    };

    peer_map.lock().unwrap().remove(&addr);
    println!("{addr} disconnected from the websocket");
}

async fn update_native_window(event_loop: EventLoopProxy<UserEvent>) {
    loop {
        event_loop.send_event(UserEvent::Redraw).unwrap();
        tokio::time::sleep(SLEEP_TIME).await;
    }
}

async fn update_peers(peer_map: PeerMap, front_buffer: &RwLock<Vec<u8>>) {
    loop {
        {
            let peer_map = peer_map.lock().unwrap();
            let bytes = Bytes::copy_from_slice(&front_buffer.read().unwrap());

            for tx in peer_map.values() {
                if tx.send(bytes.clone()).is_err() {
                    println!("Weird tx");
                }
            }
        }

        tokio::time::sleep(SLEEP_TIME).await;
    }
}

#[tokio::main]
async fn main() {
    print_available_cameras();
    let camera = make_camera();
    let size = get_camera_resolution(&camera);
    let front_buffer = RwLock::new(make_buffer(size));
    let mut app = App::new(size, &front_buffer);
    let event_loop = EventLoop::with_user_event().build().unwrap();
    let peer_map = PeerMap::new(Mutex::new(HashMap::new()));

    unsafe {
        async_scoped::TokioScope::scope(|s| {
            {
                let front_buffer = &front_buffer;
                s.spawn_blocking(move || run_camera(front_buffer));
            }

            s.spawn(run_web_server(peer_map.clone()));
            s.spawn(update_native_window(event_loop.create_proxy()));
            s.spawn(update_peers(peer_map.clone(), &front_buffer));
            event_loop.run_app(&mut app).unwrap();
        });
    }
}
