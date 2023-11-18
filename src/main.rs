use std::collections::HashSet;
use std::net::IpAddr;
use std::sync::Arc;
use std::sync::Mutex;
use std::{sync::atomic::Ordering, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};

const PORT: u16 = 2000;
const LISTEN_ADDRESS: &str = "127.0.0.1";
const EXISTANCE_BROADCAST_MILLIS: u64 = 1500;
type StdinMsg = [u8; 32];

const LISTEN_SOCKET: (&str, u16) = (LISTEN_ADDRESS, PORT);

static IN_CONNECTION: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tokio::spawn(broadcast_existance());
    let peers = Arc::new(Mutex::new(HashSet::new()));
    tokio::spawn(listen_for_peers(peers.clone()));
    tokio::spawn(print_available_peers(peers.clone()));

    let (stdin_watch_sender, stdin_watch_receiver) = tokio::sync::watch::channel([0; 32]);

    tokio::spawn(async move {
        stdin_listener(stdin_watch_sender).await.unwrap();
    });
    let stdin_watch_receiver_clone = stdin_watch_receiver.clone();
    tokio::spawn(async move {
        tcp_connection_listener(stdin_watch_receiver_clone)
            .await
            .unwrap();
    });
    tokio::spawn(async move {
        tcp_connection_creator(stdin_watch_receiver, peers.clone()).await;
    });
    loop {}
}

async fn print_available_peers(peers: Arc<Mutex<HashSet<IpAddr>>>) {
    let mut interval = tokio::time::interval(Duration::from_millis(1500));
    loop {
        interval.tick().await;
        // only print this info when not in connection
        if IN_CONNECTION.load(Ordering::Relaxed) {
            continue;
        }
        println!("Peers Available:");
        {
            let set = peers.lock().unwrap();
            for (idx, elem) in set.iter().enumerate() {
                println!("  {}: {:?}", idx, elem);
            }
        }
    }
}

async fn tcp_connection_creator(
    mut stdin_watch_receiver: tokio::sync::watch::Receiver<StdinMsg>,
    peers: Arc<Mutex<HashSet<IpAddr>>>,
) {
    loop {
        stdin_watch_receiver.changed().await.unwrap();
        let num = {
            let inp = &*stdin_watch_receiver.borrow_and_update();
            // turn u8's into utf8
            let len = inp
                .iter()
                .position(|&item| item == b'\n')
                .unwrap_or(inp.len());
            let inp = &inp[..len];
            let inp = std::str::from_utf8(inp).unwrap_or("ERRENOUS UTF8");
            println!("you just said: {:?}", inp);
            let Ok(num) = inp.parse::<u16>() else {
                println!("ERROR: {:?} is not a number", inp);
                continue;
            };
            num
        };
        let peer = {
            let set = peers.lock().unwrap();

            let Some(peer) = set.iter().skip(num as _).next() else {
                println!("ERROR: no such peer exists!");
                continue;
            };
            println!("{:?}", peer);
            peer.clone()
        };
        let Ok(mut a) = tokio::net::TcpStream::connect((peer, PORT)).await else {
            println!("ERROR: Could not connect!");
            continue;
        };
        a.write_all(b"YO").await.unwrap();
        let (read_half, write_half) = a.into_split();

        tokio::spawn(tcp_connection_receiver(read_half));
        tokio::spawn(tcp_connection_writer(
            write_half,
            stdin_watch_receiver.clone(),
        ));
    }
}

async fn tcp_connection_listener(
    stdin_watch_receiver: tokio::sync::watch::Receiver<StdinMsg>,
) -> Result<(), Box<dyn std::error::Error>> {
    let tcp = TcpListener::bind(LISTEN_SOCKET).await?;
    loop {
        let (stream, addr) = tcp.accept().await?;
        println!("got connection from: {:?}", addr);
        if IN_CONNECTION.load(Ordering::Relaxed) {
            println!("BUT already in connection");
            continue;
        }
        IN_CONNECTION.store(true, Ordering::Relaxed);

        let (tcp_read, tcp_write) = stream.into_split();

        // read from stdin and send as tcp
        let stdin_watch_receiver_clone = stdin_watch_receiver.clone();
        tokio::spawn(async move {
            tcp_connection_writer(tcp_write, stdin_watch_receiver_clone).await;
        });
        tokio::spawn(async move {
            tcp_connection_receiver(tcp_read).await;
            IN_CONNECTION.store(false, Ordering::Relaxed);
        });
    }
}

async fn tcp_connection_writer(
    mut write: tokio::net::tcp::OwnedWriteHalf,
    mut stdin_watch_receiver: tokio::sync::watch::Receiver<StdinMsg>,
) {
    loop {
        stdin_watch_receiver.changed().await.unwrap();
        let mut my_buf = [0; 32];
        stdin_watch_receiver
            .borrow_and_update()
            .clone_into(&mut my_buf);
        if write.write_all(&my_buf).await.is_err() {
            println!("exit tcp sender");
            return;
        }
    }
}

/// read from TcpListener and print to STDOUT
async fn tcp_connection_receiver(mut read: tokio::net::tcp::OwnedReadHalf) {
    let mut stdout = tokio::io::stdout();
    loop {
        let mut buf = [0; 256];
        let Ok(len) = read.read(&mut buf).await else {
            println!("exit tcplistener");
            return;
        };
        if len == 0 {
            println!("exit tcplistener");
            return;
        }
        stdout.write_all(&buf).await.unwrap();
    }
}

async fn stdin_listener(
    watch: tokio::sync::watch::Sender<StdinMsg>,
) -> Result<(), Box<dyn std::error::Error>> {
    println!("Listening on STDIN");

    let mut stdin = tokio::io::stdin();

    loop {
        let mut buf = [0; 32];
        stdin.read(&mut buf).await?;
        // println!("you wrote: {:?}", buf);
        watch.send(buf)?;
    }
}

async fn broadcast_existance() {
    let udp_sock = tokio::net::UdpSocket::bind(("0.0.0.0", 0)).await.unwrap();
    udp_sock.set_broadcast(true).unwrap();
    println!("broadcasting existance");
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
}
async fn listen_for_peers(peers: Arc<Mutex<HashSet<IpAddr>>>) {
    let udp_sock = tokio::net::UdpSocket::bind(("0.0.0.0", PORT as _))
        .await
        .unwrap();
    let mut buf = [0; 32];
    println!("listening for peers");
    loop {
        let (_size, peer) = udp_sock.recv_from(&mut buf).await.unwrap();
        println!("got udp packet from: {:?}", peer);
        tokio::io::stdout().write_all(&buf).await.unwrap();
        {
            let mut set = peers.lock().unwrap();
            set.insert(peer.ip());
        }
    }
}
