use std::collections::hash_map::ValuesMut;
use std::collections::HashMap;
use futures_util::{SinkExt, StreamExt, TryStreamExt};
use nokhwa::pixel_format::RgbAFormat;
use nokhwa::utils::{ApiBackend, CameraIndex, RequestedFormat, RequestedFormatType};
use nokhwa::Camera;
use std::net::SocketAddr;
use std::num::NonZeroU32;
use std::process::exit;
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;
use axum::extract::{ConnectInfo, WebSocketUpgrade};
use axum::extract::ws::{Message, WebSocket};
use axum::{Router, ServiceExt};
use axum::body::Bytes;
use axum::routing::{any, get};
use axum_extra::TypedHeader;
use futures_util::stream::SplitSink;
use tokio::net::{TcpListener, TcpStream};
use tower_http::services::ServeDir;
use tower_http::trace::{DefaultMakeSpan, TraceLayer};
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, EventLoop, OwnedDisplayHandle};
use winit::window::{Window, WindowId};

const CAMERA_INDEX: u32 = 1;
type Decoder = RgbAFormat;
type Tx = SplitSink<WebSocket, Message>;
type PeerMap = Arc<Mutex<HashMap<SocketAddr, Tx>>>;

struct App<'a> {
    window: Option<Arc<Window>>,
    surface: Option<softbuffer::Surface<OwnedDisplayHandle, Arc<Window>>>,
    front_buffer: &'a RwLock<Vec<u8>>,
    size: (u32, u32),
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

impl ApplicationHandler for App<'_> {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        let window_attributes = Window::default_attributes()
            .with_title("Mitcam")
            .with_inner_size::<winit::dpi::LogicalSize<u32>>(self.size.into())
            .with_resizable(false);
        self.window = Some(Arc::new(event_loop.create_window(window_attributes).unwrap()));

        let context = softbuffer::Context::new(event_loop.owned_display_handle()).unwrap();
        self.surface = Some(softbuffer::Surface::new(&context, self.window.as_ref().unwrap().clone()).unwrap());
        let (width, height) = self.size;
        self.surface.as_mut().unwrap().resize(NonZeroU32::new(width).unwrap(), NonZeroU32::new(height).unwrap()).unwrap();
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _window_id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::RedrawRequested => {
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

fn render(camera: &mut Camera, front_buffer: &RwLock<Vec<u8>>, back_buffer: &mut Vec<u8>) -> Bytes {
    camera.write_frame_to_buffer::<Decoder>(back_buffer).unwrap();
    let mut front_buffer = front_buffer.write().unwrap();
    std::mem::swap(back_buffer, front_buffer.as_mut());
    Bytes::copy_from_slice(&front_buffer)
}

fn send_bytes_to_everyone(bytes: Bytes, peer_map: ValuesMut<SocketAddr, Tx>) {
    unsafe {
        async_scoped::TokioScope::scope(|s| {
            for tx in peer_map {
                s.spawn(tx.send(Message::Binary(bytes.clone())));
            }
        });
    }
}

async fn run_camera(front_buffer: &RwLock<Vec<u8>>, peer_map: PeerMap) {
    let mut camera = make_camera();
    camera.open_stream().unwrap();
    let mut back_buffer = make_buffer(get_camera_resolution(&camera));

    loop {
        let bytes = render(&mut camera, &front_buffer, &mut back_buffer);
        send_bytes_to_everyone(bytes, peer_map.lock().unwrap().values_mut());
        std::thread::sleep(Duration::from_millis(50));
    }
}

async fn handle_connection(socket: WebSocket, addr: SocketAddr, peer_map: PeerMap) {
    println!("{addr} connected to websocket");

    let (tx, mut rx) = socket.split();
    peer_map.lock().unwrap().insert(addr, tx);

    while let Some(Ok(msg)) = rx.next().await {
        match msg {
            Message::Text(text) => println!("{} sent text: {}", addr, text),
            Message::Binary(buffer) => println!("{} sent {} bytes", addr, buffer.len()),
            Message::Close(_) => break,
            _ => {}
        }
    }

    peer_map.lock().unwrap().remove(&addr);
}

#[tokio::main]
async fn main() {
    print_available_cameras();
    let camera = make_camera();
    let size = get_camera_resolution(&camera);
    println!("Size: {size:?}");
    let front_buffer = RwLock::new(make_buffer(size));
    let mut app = App::new(size, &front_buffer);

    // tokio::spawn(async move {
    //     let socket = TcpListener::bind("0.0.0.0:5555").await.unwrap();
    //     while let Ok((stream, addr)) = socket.accept().await {
    //         tokio::spawn(handle_connection(stream, addr));
    //     }
    // });

    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| {
                format!("{}=debug,tower_http=debug", env!("CARGO_CRATE_NAME")).into()
            }),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    let peer_map = PeerMap::new(Mutex::new(HashMap::new()));

    unsafe {
        async_scoped::TokioScope::scope(|s| {
            s.spawn(async {
                let peer_map = peer_map.clone();
                let router = Router::new()
                    .route("/ws", any(async move |ws: WebSocketUpgrade, _user_agent: Option<TypedHeader<headers::UserAgent>>, ConnectInfo(addr): ConnectInfo<SocketAddr>| {
                        ws.on_upgrade(move |socket| handle_connection(socket, addr, peer_map))
                    }))
                    .fallback_service(ServeDir::new("web"))
                    .layer(TraceLayer::new_for_http().make_span_with(DefaultMakeSpan::default().include_headers(true)));

                let listener = TcpListener::bind("0.0.0.0:5555").await.unwrap();
                println!("Listening on: {}", listener.local_addr().unwrap());
                axum::serve(listener, router.into_make_service_with_connect_info::<SocketAddr>()).await.unwrap();
            });

            s.spawn(run_camera(&front_buffer, peer_map.clone()));

            let event_loop = EventLoop::new().unwrap();
            event_loop.run_app(&mut app).unwrap();

            exit(0);
        });
    }
}
