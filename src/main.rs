use axidraw_over_http::{
    axidraw_over_http_server::{AxidrawOverHttp, AxidrawOverHttpServer},
    BufferState, Command, Empty, RunningStatus,
};
use clap::Parser;
use serialport::{SerialPort, SerialPortInfo, SerialPortType};
use std::{
    collections::VecDeque,
    io::{prelude::*, BufRead, BufReader, BufWriter},
    net::IpAddr,
    str::FromStr,
    sync::Arc,
    thread::{sleep, spawn},
    time::Duration,
};
use tokio::{
    join,
    sync::{
        mpsc::{unbounded_channel, UnboundedSender},
        Mutex,
    },
};
use tokio_stream::StreamExt;
use tonic::{transport::Server, Request, Response, Status};

mod axidraw_over_http {
    tonic::include_proto!("axidraw_over_http");
}

enum ControlMessage {
    CheckBuffer,
}

struct AxidrawService {
    control_message_sender: UnboundedSender<ControlMessage>,
    command_buffer: Arc<Mutex<VecDeque<String>>>,
    running_status: Arc<Mutex<RunningStatus>>,
}

#[tonic::async_trait]
impl AxidrawOverHttp for AxidrawService {
    async fn stream(
        &self,
        request: Request<tonic::Streaming<Command>>,
    ) -> Result<Response<Empty>, Status> {
        let mut stream = request.into_inner();

        while let Some(command) = stream.next().await {
            let command = command?.contents;

            if command.is_empty() || command.contains('\r') || command.contains('\n') {
                return Err(Status::invalid_argument("Invalid command"));
            }

            self.command_buffer
                .clone()
                .lock_owned()
                .await
                .push_back(command);

            if *self.running_status.lock().await == RunningStatus::Running {
                self.control_message_sender
                    .send(ControlMessage::CheckBuffer)
                    .unwrap();
            }
        }

        Ok(Response::new(Empty {}))
    }

    async fn clear(&self, _request: Request<Empty>) -> Result<Response<Empty>, Status> {
        self.command_buffer.clone().lock_owned().await.clear();

        Ok(Response::new(Empty {}))
    }

    async fn pause(&self, _request: Request<Empty>) -> Result<Response<Empty>, Status> {
        *self.running_status.clone().lock_owned().await = RunningStatus::Paused;

        Ok(Response::new(Empty {}))
    }

    async fn resume(&self, _request: Request<Empty>) -> Result<Response<Empty>, Status> {
        *self.running_status.clone().lock_owned().await = RunningStatus::Running;
        self.control_message_sender
            .send(ControlMessage::CheckBuffer)
            .unwrap();

        Ok(Response::new(Empty {}))
    }

    async fn get_state(&self, _request: Request<Empty>) -> Result<Response<BufferState>, Status> {
        let (buffer, status) = join![self.command_buffer.lock(), self.running_status.lock()];

        return Ok(Response::new(BufferState {
            buffer_length: buffer.len() as u64,
            running_status: *status as i32,
        }));
    }
}

#[derive(Parser)]
#[command(long_about = None)]
struct Cli {
    /// Port to listen on. Defaults to 7878.
    #[arg(short, long)]
    port: Option<u16>,
    /// Serial device where the AxiDraw is connected. If none specified, will auto-detect.
    #[arg(short, long)]
    device: Option<String>,
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    let port_number = cli.port.unwrap_or(7878);

    println!("Waiting for serial connection...");
    let serial_port = get_serial_port(&cli.device);
    println!(
        "Serial connection {} opened",
        serial_port.name().unwrap_or("unknown".to_string())
    );

    let (control_message_sender, mut control_message_receiver) =
        unbounded_channel::<ControlMessage>();
    let running_status = Arc::new(Mutex::new(RunningStatus::Running));
    let command_buffer = Arc::new(Mutex::new(VecDeque::<String>::new()));

    let consumer_thread_running_status = running_status.clone();
    let consumer_thread_command_buffer = command_buffer.clone();

    spawn(move || loop {
        let control_message = control_message_receiver.blocking_recv().unwrap();

        match control_message {
            ControlMessage::CheckBuffer => loop {
                let state = consumer_thread_running_status.blocking_lock();
                let mut buffer = consumer_thread_command_buffer.clone().blocking_lock_owned();

                if *state != RunningStatus::Running || buffer.is_empty() {
                    break;
                }

                send_to_serial_and_wait_for_ok(&*serial_port, buffer.pop_front().unwrap().as_str());
            },
        }
    });

    let service = AxidrawOverHttpServer::new(AxidrawService {
        control_message_sender,
        running_status,
        command_buffer,
    });

    let server = Server::builder().add_service(service).serve_with_shutdown(
        (IpAddr::from_str("::").unwrap(), port_number).into(),
        async move {
            tokio::signal::ctrl_c().await.unwrap();
        },
    );

    let _ = tokio::task::spawn(server).await;
}

fn get_serial_port(device: &Option<String>) -> Box<dyn SerialPort> {
    let port_filter = |port_info: &&SerialPortInfo| {
        if let Some(device) = device {
            port_info.port_name == *device
        } else if let SerialPortType::UsbPort(usb_port_info) = &port_info.port_type {
            usb_port_info
                .product
                .as_ref()
                .unwrap_or(&"".to_string())
                .contains("EiBotBoard")
        } else {
            false
        }
    };

    let port_info = loop {
        let port_info = serialport::available_ports()
            .unwrap_or_default()
            .iter()
            .find(port_filter)
            .cloned();

        if let Some(port_info) = port_info {
            break port_info;
        } else {
            sleep(Duration::from_secs(1));
        }
    };

    serialport::new(&port_info.port_name, 9600)
        .timeout(Duration::from_secs(1))
        .open()
        .unwrap_or_else(|_| panic!("Could not create port on {}", &port_info.port_name))
}

fn send_to_serial_and_wait_for_ok(serial_port: &dyn SerialPort, command: &str) {
    println!("Writing to serial port: {}", command);

    let mut serial_reader_lines = BufReader::new(serial_port.try_clone().unwrap()).lines();

    let mut serial_writer = BufWriter::new(serial_port.try_clone().unwrap());
    serial_writer
        .write_all(format!("{}\r", command).as_bytes())
        .unwrap();
    serial_writer.flush().unwrap();

    let response = loop {
        if let Ok(response) = serial_reader_lines.next().unwrap() {
            break response;
        }
    };

    println!("Repsonse from serial port: {}", &response);
}
