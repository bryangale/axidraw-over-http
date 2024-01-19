use serialport::{SerialPort, SerialPortType};
use std::{
    io::{prelude::*, BufRead, BufReader, BufWriter},
    net::TcpListener,
    thread::sleep,
    time::Duration,
};

fn main() {
    println!("Waiting for serial connection...");
    let serial_port = get_serial_port();
    println!(
        "Serial connection {} opened",
        serial_port.name().unwrap_or("unknown".to_string())
    );

    let tcp_listener = TcpListener::bind("0.0.0.0:7878").unwrap();

    println!("Waiting for TCP connection...");

    for tcp_stream in tcp_listener.incoming() {
        println!("TCP connection opened");

        let tcp_stream = tcp_stream.unwrap();

        let tcp_reader_lines = BufReader::new(tcp_stream.try_clone().unwrap()).lines();
        let mut serial_reader_lines = BufReader::new(serial_port.try_clone().unwrap()).lines();

        let mut serial_writer = BufWriter::new(serial_port.try_clone().unwrap());
        let mut tcp_writer = BufWriter::new(tcp_stream.try_clone().unwrap());

        for line in tcp_reader_lines.take_while(|line| line.is_ok()) {
            let mut line = line.unwrap();
            println!("Request: {}", &line);

            line.push('\r');
            serial_writer.write_all(line.as_str().as_bytes()).unwrap();
            serial_writer.flush().unwrap();

            let response = serial_reader_lines.next().unwrap().unwrap();
            tcp_writer.write_all(response.as_str().as_bytes()).unwrap();

            println!("Repsonse: {}", &response);
        }

        println!("TCP connection closed. Waiting for new connection...");
    }
}

fn get_serial_port() -> Box<dyn SerialPort> {
    let port_info = loop {
        let port_info = serialport::available_ports()
            .unwrap_or_default()
            .iter()
            .find(|port_info| {
                if let SerialPortType::UsbPort(usb_port_info) = &port_info.port_type {
                    usb_port_info
                        .product
                        .as_ref()
                        .unwrap_or(&"".to_string())
                        .contains("EiBotBoard")
                } else {
                    false
                }
            })
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
        .expect(format!("Could not create port on {}", &port_info.port_name).as_str())
}
