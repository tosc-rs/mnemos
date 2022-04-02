use std::time::Duration;
use std::thread::sleep;

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
    loop {
        sleep(Duration::from_millis(500));
        let mut data = [0u8; 16];
        let mut buf = [0u8; 256];
        data.iter_mut().enumerate().for_each(|(i, b)| *b = (i as u8));
        let msg = sportty::Message { port: port_id, data: &data };
        let used = msg.encode_to(&mut buf).map_err(drop).unwrap();
        port.write_all(used)?;
        port_id = port_id.wrapping_add(1);
    }
}
