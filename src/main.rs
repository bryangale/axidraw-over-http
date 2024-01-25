use clap::Parser;
use serialport::{SerialPort, SerialPortInfo, SerialPortType};
use std::{
    collections::VecDeque,
    convert::Infallible,
    io::{prelude::*, BufRead, BufReader, BufWriter},
    str::FromStr,
    sync::Arc,
    thread::{sleep, spawn},
    time::Duration,
};
use tokio::sync::Mutex;
use warp::{
    reject::Rejection,
    reply::{Reply, WithStatus},
    Filter,
};

#[derive(Parser)]
#[command(long_about = None)]
struct Cli {
    /// Port to listen on. Defaults to 7878.
    #[arg(short, long)]
    port: Option<u16>,
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    let port_number = cli.port.unwrap_or(7878);

    println!("Waiting for serial connection...");
    let serial_port = get_serial_port();
    println!(
        "Serial connection {} opened",
        serial_port.name().unwrap_or("unknown".to_string())
    );

    let command_buffer = Arc::new(Mutex::new(VecDeque::<String>::new()));
    let plotter_handler_command_buffer = command_buffer.clone();

    spawn(move || loop {
        let mut command_buffer = command_buffer.blocking_lock();

        let command = command_buffer.pop_front();
        drop(command_buffer);

        if let Some(command) = command {
            send_to_serial_and_wait_for_ok(&*serial_port, command.as_str());
        }
    });

    let plotter_handler = create_plotter_handler(plotter_handler_command_buffer);

    let (_, server) = warp::serve(plotter_handler).bind_with_graceful_shutdown(
        ([0, 0, 0, 0], port_number),
        async move {
            tokio::signal::ctrl_c().await.unwrap();
        },
    );

    let _ = tokio::task::spawn(server).await;
}

fn get_serial_port() -> Box<dyn SerialPort> {
    let port_filter = |port_info: &&SerialPortInfo| {
        if let SerialPortType::UsbPort(usb_port_info) = &port_info.port_type {
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

fn create_plotter_handler(
    command_buffer: Arc<Mutex<VecDeque<String>>>,
) -> impl warp::Filter<Extract = (WithStatus<impl Reply>,), Error = Rejection> + Clone {
    async fn handler(
        command_bytes: warp::hyper::body::Bytes,
        command_buffer: Arc<Mutex<VecDeque<String>>>,
    ) -> Result<WithStatus<impl Reply>, Infallible> {
        if let Ok(body_bytes) = String::from_utf8(command_bytes.to_vec()) {
            if body_bytes.contains('\r') {
                Ok(warp::reply::with_status(
                    warp::reply(),
                    warp::http::StatusCode::BAD_REQUEST,
                ))
            } else {
                let mut command_buffer = command_buffer.lock().await;
                for command in body_bytes.split('\n') {
                    if !command.is_empty() {
                        command_buffer.push_back(String::from_str(command).unwrap());
                    }
                }

                Ok(warp::reply::with_status(
                    warp::reply(),
                    warp::http::StatusCode::OK,
                ))
            }
        } else {
            Ok(warp::reply::with_status(
                warp::reply(),
                warp::http::StatusCode::BAD_REQUEST,
            ))
        }
    }

    warp::post()
        .and(warp::path("batch-queue"))
        .and(warp::filters::body::bytes())
        .and(warp::any().map(move || command_buffer.clone()))
        .and_then(handler)
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
