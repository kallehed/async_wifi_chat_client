use std::time::Duration;

use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};

const PORT: u64 = 2000;

const EXISTANCE_BROADCAST_MILLIS: u64 = 1500;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("Hello, world!");

    // send udp broadcast messages, receive them as well

    tokio::spawn(async move {
        let udp_sock = tokio::net::UdpSocket::bind(("0.0.0.0", 0)).await.unwrap();
        udp_sock.set_broadcast(true).unwrap();
        loop {
            udp_sock
                .send_to(
                    b"YOLO OMEGA SWAG COOLIO MOORIO",
                    ("255.255.255.255", PORT as _),
                )
                .await
                .unwrap();
            tokio::time::sleep(Duration::from_millis(EXISTANCE_BROADCAST_MILLIS)).await;
        }
    });
    tokio::spawn(async move {
        let udp_sock = tokio::net::UdpSocket::bind(("0.0.0.0", PORT as _))
            .await
            .unwrap();
        let mut buf = [0; 256];
        loop {
            let (_size, peer) = udp_sock.recv_from(&mut buf).await.unwrap();
            println!("got udp packet from: {:?}", peer);
            tokio::io::stdout().write_all(&buf).await.unwrap();
        }
    });

    let tcp = TcpListener::bind("127.0.0.1:8000").await?;
    loop {
        let (mut stream, addr) = tcp.accept().await?;
        println!("got connection from: {:?}", addr);

        tokio::spawn(async move {
            loop {
                if let Err(_) = stream.write_all(b"YO what you doin").await {
                    return;
                };
                let mut buf = [0; 16];
                stream.read(&mut buf).await.unwrap();
                println!("got: {:?}", buf);
                tokio::io::stdout().write_all(&buf).await.unwrap();
                if let Err(_) = stream.write_all(&buf).await {
                    return;
                };
            }
        });
    }
}
