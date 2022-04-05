use std::io::ErrorKind;
use std::time::Duration;
use std::thread::sleep;

use sportty::Message;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut dport = None;

    for port in serialport::available_ports().unwrap() {
        if let serialport::SerialPortType::UsbPort(serialport::UsbPortInfo {
            serial_number: Some(sn),
            ..
        }) = &port.port_type
        {
            if sn.as_str() == "ajm001" {
                dport = Some(port.clone());
                break;
            }
        }
    }

    let dport = if let Some(port) = dport {
        port
    } else {
        eprintln!("Error: No `Pellegrino` connected!");
        return Ok(());
    };

    let mut port = serialport::new(dport.port_name, 115200)
        .timeout(Duration::from_millis(5))
        .open()
        .map_err(|_| "Error: failed to create port")?;

    let mut port_id = 0u16;
    let mut buf = [0u8; 128];
    let mut carry = Vec::new();

    port.set_timeout(Duration::from_millis(10)).ok();

    loop {
        sleep(Duration::from_millis(333));
        let mut data = [0u8; 16];
        let mut buf = [0u8; 256];
        data.iter_mut().enumerate().for_each(|(i, b)| *b = (i as u8));
        let msg = sportty::Message { port: port_id, data: &data };
        let used = msg.encode_to(&mut buf).map_err(drop).unwrap();
        port.write_all(used)?;
        // port_id = (port_id + 1) % 4;

        let used = match port.read(&mut buf) {
            Err(e) if e.kind() == ErrorKind::WouldBlock => continue,
            Err(e) if e.kind() == ErrorKind::TimedOut => continue,
            Ok(0) => continue,
            Ok(used) => used,
            Err(e) => panic!("{:?}", e),
        };
        carry.extend_from_slice(&buf[..used]);

        if let Some(pos) = carry.iter().position(|b| *b == 0) {
            let new_chunk = carry.split_off(pos + 1);
            if let Ok(msg) = Message::decode_in_place(&mut carry) {
                println!("Got message: {} - {:?}", msg.port, msg.data);
            } else {
                println!("Bad decode!");
            }
            carry = new_chunk;
        }
    }
}
