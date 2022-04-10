use std::{net::TcpStream, io::{Read, Write}, thread::sleep, time::Duration};

fn main() {
    let mut args = std::env::args();
    let _ = args.next();
    let ip = args.next().unwrap();
    let port = args.next().unwrap();
    let bin = args.next().unwrap();

    let dest = format!("{}:{}", ip, port);
    let mut conn = TcpStream::connect(&dest).unwrap();
    println!("Connected to '{}'.", dest);
    let mut file = std::fs::File::open(bin).unwrap();
    let mut contents = Vec::new();
    file.read_to_end(&mut contents).unwrap();
    println!("Loaded file. {} bytes.", contents.len());

    while (contents.len() % 256) != 0 {
        contents.push(0xFF);
    }

    for ch in contents.chunks_exact(64) {
        conn.write_all(ch).unwrap();
        conn.flush().unwrap();
        sleep(Duration::from_millis(250));
    }

    println!("Done.");
}
